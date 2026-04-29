use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{
    LlmContentBlock, LlmMessage, ProviderEvent, ProviderRequest, ThinkingLevel,
};
use crate::providers::{LlmProvider, ProviderError};

/// OpenAI Responses API provider adapter.
///
/// Reads `OPENAI_API_KEY` from the environment. Optionally overrides the
/// base URL via `OPENAI_BASE_URL` (defaults to
/// `https://api.openai.com/v1/responses`).
pub struct OpenAiResponsesProvider {
    api_key: Option<String>,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAiResponsesProvider {
    pub fn new() -> Self {
        let api_key = std::env::var("OPENAI_API_KEY").ok();
        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1/responses".to_string());
        let client = reqwest::Client::new();
        Self {
            api_key,
            base_url,
            client,
        }
    }

    /// Construct with an explicit API key, useful for tests and embedded callers.
    pub fn with_api_key(api_key: Option<String>) -> Self {
        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1/responses".to_string());
        let client = reqwest::Client::new();
        Self {
            api_key,
            base_url,
            client,
        }
    }

    /// Check whether an API key is configured.
    pub fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }
}

impl Default for OpenAiResponsesProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmProvider for OpenAiResponsesProvider {
    async fn stream(
        &self,
        request: ProviderRequest,
        cancel: CancellationToken,
    ) -> Result<mpsc::UnboundedReceiver<ProviderEvent>, ProviderError> {
        let api_key = self
            .api_key
            .as_deref()
            .ok_or_else(|| ProviderError::ApiKeyMissing("openai".to_string()))?;

        // Check cancellation before spawning
        if cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }

        let (tx, rx) = mpsc::unbounded_channel();
        let api_key = api_key.to_string();
        let base_url = self.base_url.clone();
        let client = self.client.clone();

        tokio::spawn(async move {
            let body = build_request_body(&request);

            let response =
                match send_with_retries(&client, &base_url, "OpenAI", &api_key, &body, &cancel)
                    .await
                {
                    Ok(resp) => resp,
                    Err(message) => {
                        let _ = tx.send(ProviderEvent::Error { message });
                        return;
                    }
                };

            if let Err(e) = process_sse_stream(response, &tx, &cancel).await {
                let _ = tx.send(ProviderEvent::Error {
                    message: format!("OpenAI stream error: {e}"),
                });
            }
        });

        Ok(rx)
    }
}

// ---------------------------------------------------------------------------
// Request body builder
// ---------------------------------------------------------------------------

use super::retry::send_with_retries;

fn build_request_body(request: &ProviderRequest) -> serde_json::Value {
    let mut body = serde_json::Map::new();

    body.insert(
        "model".to_string(),
        serde_json::Value::String(request.model.id.clone()),
    );

    if !request.system_prompt.is_empty() {
        body.insert(
            "instructions".to_string(),
            serde_json::Value::String(request.system_prompt.clone()),
        );
    }

    let input_items = build_input_items(&request.messages);
    body.insert("input".to_string(), serde_json::Value::Array(input_items));

    if !request.tools.is_empty() {
        let tools: Vec<serde_json::Value> = request
            .tools
            .iter()
            .map(|td| {
                serde_json::json!({
                    "type": "function",
                    "name": td.name,
                    "description": td.description,
                    "parameters": td.input_schema,
                })
            })
            .collect();
        body.insert("tools".to_string(), serde_json::Value::Array(tools));
    }

    body.insert("stream".to_string(), serde_json::Value::Bool(true));

    if request.model.supports_reasoning {
        let effort = match request.thinking_level {
            ThinkingLevel::Low => "low",
            ThinkingLevel::Medium => "medium",
            ThinkingLevel::High => "high",
        };
        body.insert(
            "reasoning".to_string(),
            serde_json::json!({ "effort": effort }),
        );
    }

    serde_json::Value::Object(body)
}

fn build_input_items(messages: &[LlmMessage]) -> Vec<serde_json::Value> {
    let mut items = Vec::new();

    for msg in messages {
        if msg.role == "system" {
            // The Responses API handles system via `instructions`; skip
            // system messages from the input array.
            continue;
        }

        let content_blocks: Vec<serde_json::Value> = msg
            .content
            .iter()
            .map(|block| match block {
                LlmContentBlock::Text { text } => {
                    serde_json::json!({"type": "input_text", "text": text})
                }
                LlmContentBlock::Thinking { text } => {
                    serde_json::json!({"type": "input_text", "text": format!("[reasoning]\n{text}")})
                }
                LlmContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                } => {
                    let args_str = serde_json::to_string(arguments).unwrap_or_default();
                    serde_json::json!({
                        "type": "function_call",
                        "id": id,
                        "name": name,
                        "arguments": args_str,
                    })
                }
                LlmContentBlock::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                } => {
                    serde_json::json!({
                        "type": "tool_result",
                        "tool_call_id": tool_call_id,
                        "content": content,
                        "is_error": is_error,
                    })
                }
            })
            .collect();

        let role = if msg.role == "tool" {
            "user"
        } else {
            msg.role.as_str()
        };
        items.push(serde_json::json!({
            "role": role,
            "content": content_blocks,
        }));
    }

    items
}

// ---------------------------------------------------------------------------
// SSE stream processor
// ---------------------------------------------------------------------------

async fn process_sse_stream(
    response: reqwest::Response,
    tx: &mpsc::UnboundedSender<ProviderEvent>,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let mut stream = response.bytes_stream();
    let mut buf = Vec::<u8>::new();
    let mut current_event = String::new();

    while let Some(chunk_result) = stream.next().await {
        if cancel.is_cancelled() {
            break;
        }

        let chunk = chunk_result.map_err(|e| format!("network read error: {e}"))?;
        buf.extend_from_slice(&chunk);

        let mut consumed = 0usize;
        for i in 0..buf.len() {
            if buf[i] == b'\n' {
                let line_bytes = &buf[consumed..i];
                consumed = i + 1;
                let line = std::str::from_utf8(line_bytes).unwrap_or("");
                process_line(line, &mut current_event, tx);
            }
        }

        if consumed > 0 {
            buf.drain(..consumed);
        }
    }

    // Process any trailing data that lacked a final newline
    if !buf.is_empty() {
        let line = std::str::from_utf8(&buf).unwrap_or("");
        process_line(line, &mut current_event, tx);
    }

    // Always send Completed to prevent the agent loop from hanging
    let _ = tx.send(ProviderEvent::Completed);

    Ok(())
}

fn process_line(line: &str, current_event: &mut String, tx: &mpsc::UnboundedSender<ProviderEvent>) {
    if line.starts_with("event: ") {
        *current_event = line["event: ".len()..].trim().to_string();
        return;
    }

    if let Some(data) = line.strip_prefix("data: ") {
        let data_trimmed = data.trim();

        if data_trimmed == "[DONE]" {
            return;
        }

        let json: serde_json::Value = match serde_json::from_str(data_trimmed) {
            Ok(v) => v,
            Err(_) => {
                let _ = tx.send(ProviderEvent::Error {
                    message: format!("malformed SSE data, ignoring: {data_trimmed}"),
                });
                current_event.clear();
                return;
            }
        };

        let event_type = if current_event.is_empty() {
            json["type"].as_str().unwrap_or("")
        } else {
            current_event.as_str()
        };

        dispatch_event(event_type, &json, tx);
        current_event.clear();
    }

    // Empty lines and comments are silently ignored
}

fn dispatch_event(
    event_type: &str,
    data: &serde_json::Value,
    tx: &mpsc::UnboundedSender<ProviderEvent>,
) {
    match event_type {
        // --- Text deltas ---------------------------------------------------
        "response.output_text.delta" | "output_text.delta" => {
            if let Some(delta) = data.get("delta").and_then(|v| v.as_str()) {
                if !delta.is_empty() {
                    let _ = tx.send(ProviderEvent::TextDelta {
                        text: delta.to_string(),
                    });
                }
            }
        }

        // --- Reasoning / thinking deltas -----------------------------------
        "response.reasoning_text.delta" | "reasoning_text.delta" => {
            if let Some(delta) = data.get("delta").and_then(|v| v.as_str()) {
                if !delta.is_empty() {
                    let _ = tx.send(ProviderEvent::ThinkingDelta {
                        text: delta.to_string(),
                    });
                }
            }
        }

        "response.reasoning_text.done" | "reasoning_text.done" => {
            let _ = tx.send(ProviderEvent::ThinkingEnd);
        }

        // --- Tool call started ---------------------------------------------
        "response.output_item.added" | "output_item.added" => {
            if data.get("type").and_then(|v| v.as_str()) == Some("function_call") {
                let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if !id.is_empty() && !name.is_empty() {
                    let _ = tx.send(ProviderEvent::ToolCallStarted {
                        id: id.to_string(),
                        name: name.to_string(),
                    });
                }
            }
        }

        // --- Tool call arguments delta -------------------------------------
        "response.function_call_arguments.delta" | "function_call_arguments.delta" => {
            let id = data.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
            let delta = data.get("delta").and_then(|v| v.as_str()).unwrap_or("");
            if !id.is_empty() && !delta.is_empty() {
                let _ = tx.send(ProviderEvent::ToolCallArgumentsDelta {
                    id: id.to_string(),
                    delta: delta.to_string(),
                });
            }
        }

        // --- Tool call completed -------------------------------------------
        "response.function_call_arguments.done" | "function_call_arguments.done" => {
            let id = data.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
            if !id.is_empty() {
                let _ = tx.send(ProviderEvent::ToolCallCompleted { id: id.to_string() });
            }
        }

        "response.output_item.done" | "output_item.done" => {
            if data.get("type").and_then(|v| v.as_str()) == Some("function_call") {
                let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
                if !id.is_empty() {
                    let _ = tx.send(ProviderEvent::ToolCallCompleted { id: id.to_string() });
                }
            }
        }

        // --- Completed -----------------------------------------------------
        "response.completed" | "completed" => {
            if let Some(usage) = data.get("response").and_then(|r| r.get("usage")) {
                let input_tokens = usage
                    .get("input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                let output_tokens = usage
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                let _ = tx.send(ProviderEvent::Usage {
                    input_tokens,
                    output_tokens,
                });
            }
            let _ = tx.send(ProviderEvent::Completed);
        }

        // --- Error / failure -----------------------------------------------
        "response.failed" | "failed" | "response.incomplete" | "incomplete" | "error" => {
            let msg = data
                .get("message")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    data.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("provider returned error")
                .to_string();
            let _ = tx.send(ProviderEvent::Error { message: msg });
        }

        // --- Unknown events are silently ignored ---------------------------
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::types::ModelInfo;

    fn test_request() -> ProviderRequest {
        ProviderRequest {
            model: ModelInfo {
                provider: "openai".into(),
                id: "gpt-5.5".into(),
                display_name: String::new(),
                context_window: 128_000,
                supports_reasoning: false,
                supports_tools: true,
            },
            system_prompt: "You are a test assistant.".into(),
            messages: vec![LlmMessage {
                role: "user".into(),
                content: vec![LlmContentBlock::Text {
                    text: "Hello".into(),
                }],
            }],
            tools: vec![],
            thinking_level: ThinkingLevel::Medium,
            cwd: std::path::PathBuf::from("/tmp"),
        }
    }

    #[test]
    fn test_build_request_body_basic() {
        let req = test_request();
        let body = build_request_body(&req);

        assert_eq!(body["model"], "gpt-5.5");
        assert_eq!(body["instructions"], "You are a test assistant.");
        assert_eq!(body["stream"], true);
        assert!(
            body.get("reasoning").is_none(),
            "no reasoning when supports_reasoning=false"
        );
        assert_eq!(body["input"].as_array().unwrap().len(), 1);
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][0]["content"][0]["text"], "Hello");
    }

    #[test]
    fn test_build_request_body_with_reasoning() {
        let mut req = test_request();
        req.model.supports_reasoning = true;
        req.thinking_level = ThinkingLevel::High;

        let body = build_request_body(&req);
        assert_eq!(body["reasoning"]["effort"], "high");
    }

    #[test]
    fn test_build_request_body_skips_system_messages() {
        let req = ProviderRequest {
            model: ModelInfo {
                provider: "openai".into(),
                id: "gpt-5.5".into(),
                display_name: String::new(),
                context_window: 128_000,
                supports_reasoning: false,
                supports_tools: true,
            },
            system_prompt: "You are a test assistant.".into(),
            messages: vec![
                LlmMessage {
                    role: "system".into(),
                    content: vec![LlmContentBlock::Text {
                        text: "You are helpful.".into(),
                    }],
                },
                LlmMessage {
                    role: "user".into(),
                    content: vec![LlmContentBlock::Text { text: "Hi".into() }],
                },
            ],
            tools: vec![],
            thinking_level: ThinkingLevel::Low,
            cwd: std::path::PathBuf::from("/tmp"),
        };

        let body = build_request_body(&req);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 1, "system messages are filtered from input");
        assert_eq!(input[0]["role"], "user");
    }

    #[test]
    fn test_build_request_body_includes_tools() {
        let mut req = test_request();
        req.tools = vec![crate::protocol::types::ToolDefinition {
            name: "read".into(),
            description: "Read a file".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                }
            }),
        }];

        let body = build_request_body(&req);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "read");
        assert_eq!(tools[0]["type"], "function");
    }

    #[test]
    fn test_dispatch_text_delta() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let json = serde_json::json!({"delta": "Hello", "index": 0});

        dispatch_event("response.output_text.delta", &json, &tx);
        drop(tx);

        let event = rx.try_recv().unwrap();
        match event {
            ProviderEvent::TextDelta { text } => assert_eq!(text, "Hello"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn test_dispatch_thinking_delta() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let json = serde_json::json!({"delta": "thinking...", "index": 0});

        dispatch_event("response.reasoning_text.delta", &json, &tx);
        drop(tx);

        let event = rx.try_recv().unwrap();
        match event {
            ProviderEvent::ThinkingDelta { text } => assert_eq!(text, "thinking..."),
            other => panic!("expected ThinkingDelta, got {other:?}"),
        }
    }

    #[test]
    fn test_dispatch_thinking_end() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let json = serde_json::json!({"type": "reasoning_text.done"});

        dispatch_event("response.reasoning_text.done", &json, &tx);
        drop(tx);

        let event = rx.try_recv().unwrap();
        assert!(matches!(event, ProviderEvent::ThinkingEnd));
    }

    #[test]
    fn test_dispatch_tool_call_started() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let json = serde_json::json!({
            "type": "function_call",
            "id": "call_abc",
            "name": "read",
            "arguments": "",
            "status": "in_progress"
        });

        dispatch_event("response.output_item.added", &json, &tx);
        drop(tx);

        let event = rx.try_recv().unwrap();
        match event {
            ProviderEvent::ToolCallStarted { id, name } => {
                assert_eq!(id, "call_abc");
                assert_eq!(name, "read");
            }
            other => panic!("expected ToolCallStarted, got {other:?}"),
        }
    }

    #[test]
    fn test_dispatch_tool_call_args_delta() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let json = serde_json::json!({
            "item_id": "call_abc",
            "delta": "{\"pat"
        });

        dispatch_event("response.function_call_arguments.delta", &json, &tx);
        drop(tx);

        let event = rx.try_recv().unwrap();
        match event {
            ProviderEvent::ToolCallArgumentsDelta { id, delta } => {
                assert_eq!(id, "call_abc");
                assert_eq!(delta, "{\"pat");
            }
            other => panic!("expected ToolCallArgumentsDelta, got {other:?}"),
        }
    }

    #[test]
    fn test_dispatch_tool_call_completed() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let json = serde_json::json!({"item_id": "call_abc"});

        dispatch_event("response.function_call_arguments.done", &json, &tx);
        drop(tx);

        let event = rx.try_recv().unwrap();
        assert!(matches!(event, ProviderEvent::ToolCallCompleted { .. }));
    }

    #[test]
    fn test_dispatch_completed_with_usage() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let json = serde_json::json!({
            "response": {
                "id": "resp_1",
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 20
                }
            }
        });

        dispatch_event("response.completed", &json, &tx);
        drop(tx);

        let usage_event = rx.try_recv().unwrap();
        match usage_event {
            ProviderEvent::Usage {
                input_tokens,
                output_tokens,
            } => {
                assert_eq!(input_tokens, 10);
                assert_eq!(output_tokens, 20);
            }
            other => panic!("expected Usage, got {other:?}"),
        }

        let completed_event = rx.try_recv().unwrap();
        assert!(matches!(completed_event, ProviderEvent::Completed));
    }

    #[test]
    fn test_dispatch_error() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let json = serde_json::json!({"message": "Rate limit exceeded"});

        dispatch_event("response.failed", &json, &tx);
        drop(tx);

        let event = rx.try_recv().unwrap();
        match event {
            ProviderEvent::Error { message } => {
                assert_eq!(message, "Rate limit exceeded");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn test_dispatch_error_nested() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let json = serde_json::json!({
            "error": {
                "message": "Invalid API key",
                "type": "authentication_error"
            }
        });

        dispatch_event("error", &json, &tx);
        drop(tx);

        let event = rx.try_recv().unwrap();
        match event {
            ProviderEvent::Error { message } => {
                assert_eq!(message, "Invalid API key");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn test_dispatch_unknown_event_is_ignored() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let json = serde_json::json!({"some": "data"});

        dispatch_event("some.unknown.event", &json, &tx);
        drop(tx);

        assert!(rx.try_recv().is_err(), "unknown events should be ignored");
    }

    #[test]
    fn test_process_line_basic_sse() {
        let (tx, mut _rx) = mpsc::unbounded_channel();
        let mut current_event = String::new();

        process_line("event: response.output_text.delta", &mut current_event, &tx);
        process_line(
            r#"data: {"delta":"Hello","index":0}"#,
            &mut current_event,
            &tx,
        );
        process_line("", &mut current_event, &tx);
        process_line("event: response.completed", &mut current_event, &tx);
        process_line(
            r#"data: {"response":{"id":"resp_1","usage":{"input_tokens":1,"output_tokens":2}}}"#,
            &mut current_event,
            &tx,
        );

        drop(tx);

        // Collect all events
        let mut collected = Vec::new();
        while let Ok(e) = _rx.try_recv() {
            collected.push(e);
        }

        assert_eq!(
            collected.len(),
            3,
            "expected 3 events: text delta, usage, completed"
        );
        assert!(matches!(&collected[0], ProviderEvent::TextDelta { text } if text == "Hello"));
        assert!(matches!(&collected[1], ProviderEvent::Usage { .. }));
        assert!(matches!(&collected[2], ProviderEvent::Completed));
    }

    #[test]
    fn test_missing_api_key_returns_error() {
        std::env::remove_var("OPENAI_API_KEY");

        let provider = OpenAiResponsesProvider::new();
        let request = test_request();
        let cancel = CancellationToken::new();

        let result = futures::executor::block_on(provider.stream(request, cancel));
        match result {
            Err(ProviderError::ApiKeyMissing(provider_name)) => {
                assert_eq!(provider_name, "openai");
            }
            other => panic!("expected ApiKeyMissing error, got {other:?}"),
        }
    }
}
