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
#[path = "../../tests/inline/providers_openai_responses_tests.rs"]
mod tests;
