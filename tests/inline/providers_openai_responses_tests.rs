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
