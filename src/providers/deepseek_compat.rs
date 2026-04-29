use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{
    LlmContentBlock, LlmMessage, ProviderEvent, ProviderRequest, ToolDefinition,
};
use crate::providers::{LlmProvider, ProviderError};

/// Adapter for DeepSeek-compatible Chat Completions API (`deepseek` provider).
#[derive(Debug, Clone)]
pub struct DeepSeekCompatProvider {
    api_key: Option<String>,
    base_url: String,
    client: reqwest::Client,
}

impl DeepSeekCompatProvider {
    pub fn new() -> Self {
        let api_key = std::env::var("DEEPSEEK_API_KEY").ok();
        let base_url = std::env::var("DEEPSEEK_BASE_URL")
            .unwrap_or_else(|_| "https://api.deepseek.com/chat/completions".to_string());
        Self {
            api_key,
            base_url,
            client: reqwest::Client::new(),
        }
    }

    /// Construct with an explicit API key, useful for tests and embedded callers.
    pub fn with_api_key(api_key: Option<String>) -> Self {
        let base_url = std::env::var("DEEPSEEK_BASE_URL")
            .unwrap_or_else(|_| "https://api.deepseek.com/chat/completions".to_string());
        Self {
            api_key,
            base_url,
            client: reqwest::Client::new(),
        }
    }

    /// Override the endpoint for tests or proxies.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Check whether an API key is configured.
    pub fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }
}

impl Default for DeepSeekCompatProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmProvider for DeepSeekCompatProvider {
    async fn stream(
        &self,
        request: ProviderRequest,
        cancel: CancellationToken,
    ) -> Result<mpsc::UnboundedReceiver<ProviderEvent>, ProviderError> {
        let api_key = self
            .api_key
            .clone()
            .ok_or_else(|| ProviderError::ApiKeyMissing("deepseek".to_string()))?;

        if cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }

        let url = self.base_url.clone();
        let client = self.client.clone();
        let body = build_request_body(&request);
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let response = match send_with_retries(&client, &url, &api_key, &body, &cancel).await {
                Ok(resp) => resp,
                Err(message) => {
                    let _ = tx.send(ProviderEvent::Error { message });
                    let _ = tx.send(ProviderEvent::Completed);
                    return;
                }
            };

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut open_tool_calls: HashSet<String> = HashSet::new();
            let mut index_to_id: HashMap<u64, String> = HashMap::new();

            while let Some(chunk) = stream.next().await {
                if cancel.is_cancelled() {
                    complete_open_tool_calls(&tx, &mut open_tool_calls);
                    let _ = tx.send(ProviderEvent::Completed);
                    return;
                }

                let chunk = match chunk {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        let _ = tx.send(ProviderEvent::Error {
                            message: format!("DeepSeek stream error: {e}"),
                        });
                        break;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(idx) = buffer.find('\n') {
                    let line = buffer[..idx].trim().to_string();
                    buffer = buffer[idx + 1..].to_string();
                    process_sse_line(&line, &tx, &mut open_tool_calls, &mut index_to_id);
                }
            }

            if !buffer.trim().is_empty() {
                process_sse_line(buffer.trim(), &tx, &mut open_tool_calls, &mut index_to_id);
            }
            complete_open_tool_calls(&tx, &mut open_tool_calls);
            let _ = tx.send(ProviderEvent::Completed);
        });

        Ok(rx)
    }
}

const MAX_REQUEST_ATTEMPTS: usize = 5;

async fn send_with_retries(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    body: &serde_json::Value,
    cancel: &CancellationToken,
) -> Result<reqwest::Response, String> {
    let mut last_error = String::new();

    for attempt in 1..=MAX_REQUEST_ATTEMPTS {
        if cancel.is_cancelled() {
            return Err("DeepSeek request cancelled".to_string());
        }

        let result = client
            .post(url)
            .bearer_auth(api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(body)
            .send()
            .await;

        match result {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                let status = response.status();
                let retryable = status.as_u16() == 429 || status.is_server_error();
                let body_text = response.text().await.unwrap_or_default();
                last_error = format!("DeepSeek HTTP {status}: {body_text}");
                if !retryable || attempt == MAX_REQUEST_ATTEMPTS {
                    return Err(with_attempts(last_error, attempt));
                }
            }
            Err(err) => {
                last_error = format!("DeepSeek request failed: {}", format_reqwest_error(&err));
                if attempt == MAX_REQUEST_ATTEMPTS {
                    return Err(with_attempts(last_error, attempt));
                }
            }
        }

        sleep(retry_delay(attempt)).await;
    }

    Err(with_attempts(last_error, MAX_REQUEST_ATTEMPTS))
}

fn retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(250 * attempt as u64)
}

fn with_attempts(message: String, attempts: usize) -> String {
    format!("{message} (after {attempts}/{MAX_REQUEST_ATTEMPTS} attempts)")
}

fn format_reqwest_error(err: &reqwest::Error) -> String {
    let mut parts = vec![err.to_string()];
    let mut source = err.source();
    while let Some(src) = source {
        parts.push(src.to_string());
        source = src.source();
    }
    parts.join("; source: ")
}

fn build_request_body(request: &ProviderRequest) -> serde_json::Value {
    let mut messages = vec![serde_json::json!({
        "role": "system",
        "content": request.system_prompt,
    })];

    for msg in &request.messages {
        messages.extend(convert_message(msg));
    }

    let mut body = serde_json::json!({
        "model": request.model.id,
        "messages": messages,
        "stream": true,
    });

    let tools = convert_tools(&request.tools);
    if !tools.is_empty() {
        body["tools"] = serde_json::Value::Array(tools);
    }

    body
}

fn convert_message(message: &LlmMessage) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut text_parts = Vec::new();
    let mut thinking_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for block in &message.content {
        match block {
            LlmContentBlock::Text { text } => text_parts.push(text.clone()),
            LlmContentBlock::Thinking { text } => thinking_parts.push(text.clone()),
            LlmContentBlock::ToolCall {
                id,
                name,
                arguments,
            } => {
                tool_calls.push(serde_json::json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments.to_string(),
                    }
                }));
            }
            LlmContentBlock::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => {
                out.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": if *is_error { format!("ERROR: {content}") } else { content.clone() },
                }));
            }
        }
    }

    if !text_parts.is_empty() || !thinking_parts.is_empty() || !tool_calls.is_empty() {
        let role = if message.role == "tool" {
            "user"
        } else {
            message.role.as_str()
        };
        let mut item = serde_json::json!({
            "role": role,
            "content": text_parts.join("\n"),
        });
        if role == "assistant" && !thinking_parts.is_empty() {
            // DeepSeek thinking-mode follow-up requests must echo the prior
            // assistant reasoning text as `reasoning_content`.
            item["reasoning_content"] = serde_json::Value::String(thinking_parts.join(""));
        }
        if !tool_calls.is_empty() {
            item["tool_calls"] = serde_json::Value::Array(tool_calls);
        }
        out.insert(0, item);
    }

    out
}

fn convert_tools(tools: &[ToolDefinition]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                }
            })
        })
        .collect()
}

fn process_sse_line(
    line: &str,
    tx: &mpsc::UnboundedSender<ProviderEvent>,
    open_tool_calls: &mut HashSet<String>,
    index_to_id: &mut HashMap<u64, String>,
) {
    if line.is_empty() || !line.starts_with("data:") {
        return;
    }

    let data = line.trim_start_matches("data:").trim();
    if data == "[DONE]" {
        complete_open_tool_calls(tx, open_tool_calls);
        let _ = tx.send(ProviderEvent::Completed);
        return;
    }

    let value: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("Malformed DeepSeek SSE JSON ignored: {e}");
            return;
        }
    };

    if let Some(err) = value.get("error") {
        let message = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or_else(|| err.as_str().unwrap_or("DeepSeek error"));
        let _ = tx.send(ProviderEvent::Error {
            message: message.to_string(),
        });
        return;
    }

    if let Some(usage) = value.get("usage") {
        let input_tokens = usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let output_tokens = usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let _ = tx.send(ProviderEvent::Usage {
            input_tokens,
            output_tokens,
        });
    }

    let Some(choice) = value.get("choices").and_then(|c| c.get(0)) else {
        return;
    };

    if let Some(delta) = choice.get("delta") {
        if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
            if !content.is_empty() {
                let _ = tx.send(ProviderEvent::TextDelta {
                    text: content.to_string(),
                });
            }
        }

        if let Some(reasoning) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
            if !reasoning.is_empty() {
                let _ = tx.send(ProviderEvent::ThinkingDelta {
                    text: reasoning.to_string(),
                });
            }
        }

        if let Some(calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            for call in calls {
                let idx = call.get("index").and_then(|v| v.as_u64()).unwrap_or(0);

                // Resolve the real ID: prefer the explicit "id" field, but fall
                // back to the index→id mapping (subsequent argument-only chunks
                // only carry "index", not "id").
                let id = if let Some(explicit_id) = call.get("id").and_then(|v| v.as_str()) {
                    let id = explicit_id.to_string();
                    index_to_id.insert(idx, id.clone());
                    id
                } else {
                    index_to_id
                        .get(&idx)
                        .cloned()
                        .unwrap_or_else(|| format!("call_{idx}"))
                };

                let func = call.get("function").cloned().unwrap_or_default();
                let name = func.get("name").and_then(|v| v.as_str());
                if let Some(name) = name {
                    if open_tool_calls.insert(id.clone()) {
                        let _ = tx.send(ProviderEvent::ToolCallStarted {
                            id: id.clone(),
                            name: name.to_string(),
                        });
                    }
                }
                if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                    if !args.is_empty() {
                        let _ = tx.send(ProviderEvent::ToolCallArgumentsDelta {
                            id: id.clone(),
                            delta: args.to_string(),
                        });
                    }
                }
            }
        }
    }

    if let Some(reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
        match reason {
            "tool_calls" | "stop" | "length" => {
                complete_open_tool_calls(tx, open_tool_calls);
                let _ = tx.send(ProviderEvent::Completed);
            }
            _ => {}
        }
    }
}

fn complete_open_tool_calls(
    tx: &mpsc::UnboundedSender<ProviderEvent>,
    open_tool_calls: &mut HashSet<String>,
) {
    let ids: Vec<String> = open_tool_calls.drain().collect();
    for id in ids {
        let _ = tx.send(ProviderEvent::ToolCallCompleted { id });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_explicit_api_key_constructor() {
        let provider = DeepSeekCompatProvider::with_api_key(Some("key".to_string()));
        assert!(provider.is_configured());
        let provider = DeepSeekCompatProvider::with_api_key(None);
        assert!(!provider.is_configured());
    }

    #[test]
    fn test_text_sse() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut open = HashSet::new();
        let mut idx_map = HashMap::new();
        process_sse_line(
            r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#,
            &tx,
            &mut open,
            &mut idx_map,
        );
        match rx.try_recv().unwrap() {
            ProviderEvent::TextDelta { text } => assert_eq!(text, "hello"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn test_convert_assistant_reasoning_content() {
        let request = ProviderRequest {
            model: crate::protocol::types::ModelInfo {
                provider: "deepseek".into(),
                id: "deepseek-v4-flash".into(),
                display_name: String::new(),
                context_window: 128_000,
                supports_reasoning: true,
                supports_tools: true,
            },
            system_prompt: "system".into(),
            messages: vec![LlmMessage {
                role: "assistant".into(),
                content: vec![
                    LlmContentBlock::Thinking { text: "reason".into() },
                    LlmContentBlock::ToolCall {
                        id: "call_1".into(),
                        name: "find".into(),
                        arguments: serde_json::json!({"pattern":"*.rs"}),
                    },
                ],
            }],
            tools: vec![],
            thinking_level: crate::protocol::types::ThinkingLevel::Medium,
            cwd: std::path::PathBuf::from("."),
        };
        let body = build_request_body(&request);
        let msg = &body["messages"].as_array().unwrap()[1];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["reasoning_content"], "reason");
        assert!(msg["tool_calls"].is_array());
    }

    #[test]
    fn test_tool_sse() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut open = HashSet::new();
        let mut idx_map = HashMap::new();
        process_sse_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read","arguments":"{\"path\":"}}]}}]}"#,
            &tx,
            &mut open,
            &mut idx_map,
        );
        assert!(matches!(
            rx.try_recv().unwrap(),
            ProviderEvent::ToolCallStarted { .. }
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            ProviderEvent::ToolCallArgumentsDelta { .. }
        ));
    }

    /// Reproduces the real DeepSeek streaming bug: only the first delta carries
    /// the `id` field; subsequent argument-only deltas carry only `index`.
    #[test]
    fn test_tool_sse_index_only_subsequent_deltas() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut open = HashSet::new();
        let mut idx_map = HashMap::new();

        // First delta: has id, name, and first part of arguments
        process_sse_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_00_abc123","type":"function","function":{"name":"bash","arguments":""}}]}}]}"#,
            &tx,
            &mut open,
            &mut idx_map,
        );
        // Should get ToolCallStarted (arguments empty, not sent as delta)
        assert!(matches!(
            rx.try_recv().unwrap(),
            ProviderEvent::ToolCallStarted { id, name } if id == "call_00_abc123" && name == "bash"
        ));

        // Second delta: only index, no id — just continuation of arguments
        process_sse_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"command\":"}}]}}]}"#,
            &tx,
            &mut open,
            &mut idx_map,
        );
        // Should get ToolCallArgumentsDelta with the SAME id
        match rx.try_recv().unwrap() {
            ProviderEvent::ToolCallArgumentsDelta { id, delta } => {
                assert_eq!(id, "call_00_abc123", "subsequent delta must use the real id from index→id map");
                assert_eq!(delta, "{\"command\":");
            }
            other => panic!("expected ToolCallArgumentsDelta, got {other:?}"),
        }

        // Third delta: finishing the arguments
        process_sse_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"ls\"}"}}]}}]}"#,
            &tx,
            &mut open,
            &mut idx_map,
        );
        match rx.try_recv().unwrap() {
            ProviderEvent::ToolCallArgumentsDelta { id, delta } => {
                assert_eq!(id, "call_00_abc123");
                assert_eq!(delta, "\"ls\"}");
            }
            other => panic!("expected ToolCallArgumentsDelta, got {other:?}"),
        }
    }
}
