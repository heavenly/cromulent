use std::collections::HashSet;

use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::mpsc;
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
            let res = client
                .post(url)
                .bearer_auth(api_key)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .json(&body)
                .send()
                .await;

            let response = match res {
                Ok(resp) => resp,
                Err(e) => {
                    let _ = tx.send(ProviderEvent::Error {
                        message: format!("DeepSeek request failed: {e}"),
                    });
                    let _ = tx.send(ProviderEvent::Completed);
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                let _ = tx.send(ProviderEvent::Error {
                    message: format!("DeepSeek HTTP {status}: {body}"),
                });
                let _ = tx.send(ProviderEvent::Completed);
                return;
            }

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut open_tool_calls: HashSet<String> = HashSet::new();

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
                    process_sse_line(&line, &tx, &mut open_tool_calls);
                }
            }

            if !buffer.trim().is_empty() {
                process_sse_line(buffer.trim(), &tx, &mut open_tool_calls);
            }
            complete_open_tool_calls(&tx, &mut open_tool_calls);
            let _ = tx.send(ProviderEvent::Completed);
        });

        Ok(rx)
    }
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
    let mut tool_calls = Vec::new();

    for block in &message.content {
        match block {
            LlmContentBlock::Text { text } => text_parts.push(text.clone()),
            LlmContentBlock::ToolCall { id, name, arguments } => {
                tool_calls.push(serde_json::json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments.to_string(),
                    }
                }));
            }
            LlmContentBlock::ToolResult { tool_call_id, content, is_error } => {
                out.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": if *is_error { format!("ERROR: {content}") } else { content.clone() },
                }));
            }
        }
    }

    if !text_parts.is_empty() || !tool_calls.is_empty() {
        let role = if message.role == "tool" { "user" } else { message.role.as_str() };
        let mut item = serde_json::json!({
            "role": role,
            "content": text_parts.join("\n"),
        });
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
        .map(|tool| serde_json::json!({
            "type": "function",
            "function": {
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
            }
        }))
        .collect()
}

fn process_sse_line(
    line: &str,
    tx: &mpsc::UnboundedSender<ProviderEvent>,
    open_tool_calls: &mut HashSet<String>,
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
        let _ = tx.send(ProviderEvent::Error { message: message.to_string() });
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
        let _ = tx.send(ProviderEvent::Usage { input_tokens, output_tokens });
    }

    let Some(choice) = value.get("choices").and_then(|c| c.get(0)) else {
        return;
    };

    if let Some(delta) = choice.get("delta") {
        if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
            if !content.is_empty() {
                let _ = tx.send(ProviderEvent::TextDelta { text: content.to_string() });
            }
        }

        if let Some(reasoning) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
            if !reasoning.is_empty() {
                let _ = tx.send(ProviderEvent::ThinkingDelta { text: reasoning.to_string() });
            }
        }

        if let Some(calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            for call in calls {
                let id = call
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        let idx = call.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                        format!("call_{idx}")
                    });
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
        process_sse_line(
            r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#,
            &tx,
            &mut open,
        );
        match rx.try_recv().unwrap() {
            ProviderEvent::TextDelta { text } => assert_eq!(text, "hello"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn test_tool_sse() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut open = HashSet::new();
        process_sse_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read","arguments":"{\"path\":"}}]}}]}"#,
            &tx,
            &mut open,
        );
        assert!(matches!(rx.try_recv().unwrap(), ProviderEvent::ToolCallStarted { .. }));
        assert!(matches!(rx.try_recv().unwrap(), ProviderEvent::ToolCallArgumentsDelta { .. }));
    }
}
