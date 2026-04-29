use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::oneshot;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::protocol::events::ServerEvent;
use crate::protocol::types::{
    AskOption, AskUserResponse, ContentBlock, ToolContext, ToolDefinition, ToolResult,
};
use crate::tools::registry::Tool;
use crate::tools::ToolError;
use crate::transport::writer::OutputItem;
use crate::util::ids::generate_ask_id;

/// Cloneable handle for tools and runtime to manage pending asks.
/// Wraps an Arc to the shared AskManagerState using tokio::sync::Mutex.
#[derive(Debug, Clone)]
pub struct AskManagerHandle {
    pub(crate) inner: Arc<Mutex<AskManagerState>>,
}

#[derive(Debug, Default)]
pub(crate) struct AskManagerState {
    pub pending: HashMap<String, oneshot::Sender<AskUserResponse>>,
}

impl AskManagerHandle {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(AskManagerState::default())),
        }
    }

    /// Register a pending ask and return the receiver.
    /// This is async because it acquires the tokio::sync::Mutex.
    pub async fn register(&self, ask_id: String) -> oneshot::Receiver<AskUserResponse> {
        let (tx, rx) = oneshot::channel();
        let mut state = self.inner.lock().await;
        state.pending.insert(ask_id, tx);
        rx
    }

    /// Resolve a pending ask by sending the response.
    pub async fn resolve(&self, ask_id: &str, response: AskUserResponse) -> Result<(), String> {
        let mut state = self.inner.lock().await;
        match state.pending.remove(ask_id) {
            Some(sender) => sender
                .send(response)
                .map_err(|_| "Receiver dropped".to_string()),
            None => Err(format!("Unknown askId: {ask_id}")),
        }
    }

    /// Cancel all pending asks.
    pub async fn cancel_all(&self) {
        let mut state = self.inner.lock().await;
        state.pending.clear();
    }
}

/// The ask_user tool — allows the agent to ask the user a question and await a response.
pub struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "ask_user".to_string(),
            description: "Ask the user a question and wait for their response. Use this when you need clarification, a decision, or approval before proceeding. Provide context to help the user make an informed choice.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The question to ask the user"
                    },
                    "context": {
                        "type": "string",
                        "description": "Optional context to help the user make a decision"
                    },
                    "options": {
                        "type": "array",
                        "description": "Optional multiple-choice options",
                        "items": {
                            "type": "object",
                            "properties": {
                                "title": { "type": "string" },
                                "description": { "type": "string" }
                            },
                            "required": ["title"]
                        }
                    },
                    "allowMultiple": {
                        "type": "boolean",
                        "description": "Allow selecting multiple options (default: false)"
                    },
                    "allowFreeform": {
                        "type": "boolean",
                        "description": "Allow freeform text input (default: true)"
                    },
                    "allowComment": {
                        "type": "boolean",
                        "description": "Allow a comment alongside the response (default: false)"
                    },
                    "timeoutMs": {
                        "type": "integer",
                        "description": "Timeout in milliseconds before the ask expires"
                    }
                },
                "required": ["question"]
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolContext,
        arguments: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolError> {
        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let question = arguments
            .get("question")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("Missing required 'question' argument".into())
            })?
            .to_string();

        let context = arguments
            .get("context")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let options: Vec<AskOption> = arguments
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| {
                        Some(AskOption {
                            title: o.get("title")?.as_str()?.to_string(),
                            description: o
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let allow_multiple = arguments
            .get("allowMultiple")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let allow_freeform = arguments
            .get("allowFreeform")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let allow_comment = arguments
            .get("allowComment")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let timeout_ms = arguments.get("timeoutMs").and_then(|v| v.as_u64());

        // Generate a unique ask ID
        let ask_id = generate_ask_id();

        // Register the pending ask
        let rx = ctx.ask_manager.register(ask_id.clone()).await;

        // Emit the Ask event via the event channel
        let ev = ServerEvent::Ask {
            run_id: ctx.run_id.clone(),
            id: ask_id.clone(),
            question: question.clone(),
            context: context.clone(),
            options: options.clone(),
            allow_multiple,
            allow_freeform,
            allow_comment,
            timeout_ms,
        };
        let _ = ctx.event_tx.send(OutputItem::Event(
            serde_json::to_value(ev).expect("Ask event serialization failed"),
        ));

        // Wait for the response or cancellation
        let response = tokio::select! {
            biased;

            _ = cancel.cancelled() => {
                return Err(ToolError::Cancelled);
            }

            result = rx => {
                match result {
                    Ok(response) => response,
                    Err(_oneshot_recv_error) => {
                        return Err(ToolError::AskError("Ask response channel closed unexpectedly".into()));
                    }
                }
            }
        };

        let selected = response.selected.clone();
        let freeform = response.freeform.clone();
        let comment = response.comment.clone();

        let mut text_parts: Vec<String> = Vec::new();

        if !selected.is_empty() {
            text_parts.push(format!("Selected: {}", selected.join(", ")));
        }
        if let Some(f) = &freeform {
            if !f.is_empty() {
                text_parts.push(format!("Response: {f}"));
            }
        }
        if let Some(c) = &comment {
            if !c.is_empty() {
                text_parts.push(format!("Comment: {c}"));
            }
        }

        let text = if text_parts.is_empty() {
            "User provided no response.".to_string()
        } else {
            text_parts.join("\n")
        };

        Ok(ToolResult {
            content: vec![ContentBlock::Text { text }],
            is_error: false,
            metadata: Some(serde_json::json!({
                "askId": ask_id,
                "selected": selected,
                "freeform": freeform,
                "comment": comment,
            })),
        })
    }
}
