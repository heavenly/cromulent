use super::*;

#[test]
fn test_new_provider() {
    let p = OpenAiCompatProvider::new(
        "test",
        Some("key".into()),
        "https://example.com/v1/chat/completions",
    );
    assert!(p.is_configured());
    assert_eq!(p.name(), "test");

    let p = OpenAiCompatProvider::new("test", None, "https://example.com/v1/chat/completions");
    assert!(!p.is_configured());
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
            provider: "test".into(),
            id: "test-model".into(),
            display_name: String::new(),
            context_window: 128_000,
            supports_reasoning: true,
            supports_tools: true,
        },
        system_prompt: "system".into(),
        messages: vec![LlmMessage {
            role: "assistant".into(),
            content: vec![
                LlmContentBlock::Thinking {
                    text: "reason".into(),
                },
                LlmContentBlock::ToolCall {
                    id: "call_1".into(),
                    name: "find".into(),
                    arguments: serde_json::json!({"pattern":"*.rs"}),
                },
            ],
        }],
        tools: vec![],
        thinking_level: crate::protocol::types::ThinkingLevel::Medium,
    };
    let body = build_request_body(&request);
    let msg = &body["messages"].as_array().unwrap()[1];
    assert_eq!(msg["role"], "assistant");
    assert_eq!(msg["reasoning_content"], "reason");
    assert!(msg["tool_calls"].is_array());
}

#[test]
fn test_convert_assistant_text_only_still_has_reasoning_content() {
    let request = ProviderRequest {
        model: crate::protocol::types::ModelInfo {
            provider: "test".into(),
            id: "test-model".into(),
            display_name: String::new(),
            context_window: 128_000,
            supports_reasoning: true,
            supports_tools: true,
        },
        system_prompt: "system".into(),
        messages: vec![LlmMessage {
            role: "assistant".into(),
            content: vec![LlmContentBlock::Text {
                text: "Done!".into(),
            }],
        }],
        tools: vec![],
        thinking_level: crate::protocol::types::ThinkingLevel::Medium,
    };
    let body = build_request_body(&request);
    let msg = &body["messages"].as_array().unwrap()[1];
    assert_eq!(msg["role"], "assistant");
    assert_eq!(msg["reasoning_content"], "");
    assert_eq!(msg["content"], "Done!");
}

#[test]
fn test_convert_assistant_tool_call_has_empty_reasoning_content_when_missing() {
    let request = ProviderRequest {
        model: crate::protocol::types::ModelInfo {
            provider: "test".into(),
            id: "test-model".into(),
            display_name: String::new(),
            context_window: 128_000,
            supports_reasoning: true,
            supports_tools: true,
        },
        system_prompt: "system".into(),
        messages: vec![LlmMessage {
            role: "assistant".into(),
            content: vec![LlmContentBlock::ToolCall {
                id: "call_1".into(),
                name: "find".into(),
                arguments: serde_json::json!({"pattern":"*.rs"}),
            }],
        }],
        tools: vec![],
        thinking_level: crate::protocol::types::ThinkingLevel::Medium,
    };
    let body = build_request_body(&request);
    let msg = &body["messages"].as_array().unwrap()[1];
    assert_eq!(msg["role"], "assistant");
    assert_eq!(msg["reasoning_content"], "");
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
            assert_eq!(
                id, "call_00_abc123",
                "subsequent delta must use the real id from index→id map"
            );
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
