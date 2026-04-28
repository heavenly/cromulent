use cromulent::protocol::commands::ClientCommand;
use cromulent::protocol::events::ServerEvent;
use cromulent::protocol::responses::CommandResponse;
use cromulent::protocol::types::{
    AskOption, AskUserResponse, ContentBlock, Message, MessageRole, ModelInfo, ThinkingLevel,
    UsageInfo,
};
use cromulent::session::store::SessionHeader;

// -----------------------------------------------------------------------
// ClientCommand deserialization tests
// -----------------------------------------------------------------------

#[test]
fn test_deserialize_prompt_command() {
    let json = r#"{"type":"prompt","id":"1","message":"Write a function"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::Prompt { id, message } => {
            assert_eq!(id, Some("1".into()));
            assert_eq!(message, "Write a function");
        }
        _ => panic!("Expected Prompt command"),
    }
}

#[test]
fn test_deserialize_abort_command() {
    let json = r#"{"type":"abort","id":"2"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::Abort { id } => {
            assert_eq!(id, Some("2".into()));
        }
        _ => panic!("Expected Abort command"),
    }
}

#[test]
fn test_deserialize_user_response_command() {
    let json = r#"{"type":"userResponse","id":"3","askId":"ask_1","response":{"selected":["Option A"],"freeform":"custom","comment":"ok"}}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::UserResponse {
            id,
            ask_id,
            response,
        } => {
            assert_eq!(id, Some("3".into()));
            assert_eq!(ask_id, "ask_1");
            assert_eq!(response.selected, vec!["Option A"]);
            assert_eq!(response.freeform, Some("custom".into()));
            assert_eq!(response.comment, Some("ok".into()));
        }
        _ => panic!("Expected UserResponse command"),
    }
}

#[test]
fn test_deserialize_set_model_command() {
    let json = r#"{"type":"setModel","id":"4","provider":"openai","modelId":"gpt-5-codex"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::SetModel {
            id,
            provider,
            model_id,
        } => {
            assert_eq!(id, Some("4".into()));
            assert_eq!(provider, "openai");
            assert_eq!(model_id, "gpt-5-codex");
        }
        _ => panic!("Expected SetModel command"),
    }
}

#[test]
fn test_deserialize_set_thinking_command() {
    let json = r#"{"type":"setThinking","id":"5","level":"high"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::SetThinking { id, level } => {
            assert_eq!(id, Some("5".into()));
            assert_eq!(level, ThinkingLevel::High);
        }
        _ => panic!("Expected SetThinking command"),
    }
}

#[test]
fn test_deserialize_shutdown_command() {
    let json = r#"{"type":"shutdown","id":"6"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::Shutdown { id } => {
            assert_eq!(id, Some("6".into()));
        }
        _ => panic!("Expected Shutdown command"),
    }
}

#[test]
fn test_deserialize_get_state_command() {
    let json = r#"{"type":"getState","id":"7"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::GetState { id } => {
            assert_eq!(id, Some("7".into()));
        }
        _ => panic!("Expected GetState command"),
    }
}

#[test]
fn test_deserialize_bash_command() {
    let json = r#"{"type":"bash","id":"8","command":"git status"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::Bash { id, command } => {
            assert_eq!(id, Some("8".into()));
            assert_eq!(command, "git status");
        }
        _ => panic!("Expected Bash command"),
    }
}

#[test]
fn test_deserialize_list_sessions_command() {
    let json = r#"{"type":"listSessions","id":"9"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::ListSessions { id } => {
            assert_eq!(id, Some("9".into()));
        }
        _ => panic!("Expected ListSessions command"),
    }
}

#[test]
fn test_deserialize_load_session_command() {
    let json = r#"{"type":"loadSession","id":"10","sessionId":"abc123"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::LoadSession { id, session_id } => {
            assert_eq!(id, Some("10".into()));
            assert_eq!(session_id, "abc123");
        }
        _ => panic!("Expected LoadSession command"),
    }
}

#[test]
fn test_deserialize_new_session_command() {
    let json = r#"{"type":"newSession","id":"11"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::NewSession { id } => {
            assert_eq!(id, Some("11".into()));
        }
        _ => panic!("Expected NewSession command"),
    }
}

#[test]
fn test_deserialize_fork_session_command() {
    let json = r#"{"type":"forkSession","id":"12","entryId":"msg_456"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::ForkSession { id, entry_id } => {
            assert_eq!(id, Some("12".into()));
            assert_eq!(entry_id, "msg_456");
        }
        _ => panic!("Expected ForkSession command"),
    }
}

#[test]
fn test_deserialize_export_session_command() {
    let json = r#"{"type":"exportSession","id":"13","outputPath":"/tmp/session.json"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::ExportSession { id, output_path } => {
            assert_eq!(id, Some("13".into()));
            assert_eq!(output_path, "/tmp/session.json");
        }
        _ => panic!("Expected ExportSession command"),
    }
}

#[test]
fn test_deserialize_cycle_model_command() {
    let json = r#"{"type":"cycleModel","id":"14"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::CycleModel { id } => {
            assert_eq!(id, Some("14".into()));
        }
        _ => panic!("Expected CycleModel command"),
    }
}

#[test]
fn test_deserialize_command_without_id() {
    let json = r#"{"type":"prompt","message":"hello"}"#;
    let cmd: ClientCommand = serde_json::from_str(json).unwrap();
    match cmd {
        ClientCommand::Prompt { id, message } => {
            assert_eq!(id, None);
            assert_eq!(message, "hello");
        }
        _ => panic!("Expected Prompt command"),
    }
}

// -----------------------------------------------------------------------
// CommandResponse serialization tests
// -----------------------------------------------------------------------

#[test]
fn test_serialize_response_ok() {
    let resp = CommandResponse::ok(Some("1".into()));
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains(r#""id":"1""#));
    assert!(json.contains(r#""success":true"#));
    assert!(!json.contains("error"), "ok response should not have error field: {json}");
}

#[test]
fn test_serialize_response_err() {
    let resp = CommandResponse::err(Some("2".into()), "Something went wrong");
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains(r#""id":"2""#), "json should contain id: {json}");
    assert!(json.contains(r#""success":false"#), "json should contain success=false: {json}");
    assert!(json.contains(r#""error":"Something went wrong""#), "json should contain error: {json}");
}

#[test]
fn test_serialize_response_ok_with_data() {
    let data = serde_json::json!({ "runId": "run_abc" });
    let resp = CommandResponse::ok_with_data(Some("3".into()), data);
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains(r#""runId":"run_abc""#), "data should contain runId: {json}");
    assert!(json.contains(r#""success":true"#), "should have success=true: {json}");
}

// -----------------------------------------------------------------------
// ServerEvent serialization tests
// -----------------------------------------------------------------------

#[test]
fn test_serialize_session_changed_event() {
    let event = ServerEvent::SessionChanged {
        session_id: "ses_abc".into(),
        cwd: "/proj".into(),
        model: ModelInfo {
            provider: "openai".into(),
            id: "gpt-5.5".into(),
            display_name: "GPT-5.5".into(),
            context_window: 128_000,
            supports_reasoning: false,
            supports_tools: true,
        },
        thinking_level: ThinkingLevel::Medium,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"sessionChanged""#));
    assert!(json.contains(r#""sessionId":"ses_abc""#));
    assert!(json.contains(r#""cwd":"/proj""#));
    assert!(json.contains(r#""provider":"openai""#));
}

#[test]
fn test_serialize_agent_start_event() {
    let event = ServerEvent::AgentStart {
        run_id: "run_1".into(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"agentStart""#));
    assert!(json.contains(r#""runId":"run_1""#));
}

#[test]
fn test_serialize_turn_start_event() {
    let event = ServerEvent::TurnStart {
        run_id: "run_1".into(),
        turn: 1,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"turnStart""#));
    assert!(json.contains(r#""turn":1"#));
}

#[test]
fn test_serialize_text_delta_event() {
    let event = ServerEvent::TextDelta {
        run_id: "run_1".into(),
        text: "Hello ".into(),
        partial: "Hello ".into(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"textDelta""#));
    assert!(json.contains(r#""text":"Hello ""#));
}

#[test]
fn test_serialize_thinking_delta_event() {
    let event = ServerEvent::ThinkingDelta {
        run_id: "run_1".into(),
        text: "Let me analyze".into(),
        partial: "Let me analyze".into(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"thinkingDelta""#));
    assert!(json.contains(r#""text":"Let me analyze""#));
}

#[test]
fn test_serialize_thinking_end_event() {
    let event = ServerEvent::ThinkingEnd {
        run_id: "run_1".into(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"thinkingEnd""#));
}

#[test]
fn test_serialize_tool_call_event() {
    let event = ServerEvent::ToolCall {
        run_id: "run_1".into(),
        id: "call_1".into(),
        name: "read".into(),
        arguments: serde_json::json!({"path": "src/main.rs"}),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"toolCall""#));
    assert!(json.contains(r#""id":"call_1""#));
    assert!(json.contains(r#""name":"read""#));
    assert!(json.contains(r#""path":"src/main.rs""#));
}

#[test]
fn test_serialize_tool_result_event() {
    let event = ServerEvent::ToolResult {
        run_id: "run_1".into(),
        tool_call_id: "call_1".into(),
        content: vec![ContentBlock::Text {
            text: "file contents".into(),
        }],
        is_error: false,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"toolResult""#));
    assert!(json.contains(r#""toolCallId":"call_1""#));
    assert!(json.contains(r#""text":"file contents""#));
}

#[test]
fn test_serialize_ask_event() {
    let event = ServerEvent::Ask {
        run_id: "run_1".into(),
        id: "ask_1".into(),
        question: "Which approach?".into(),
        context: Some("We have two options".into()),
        options: vec![
            AskOption {
                title: "A".into(),
                description: Some("Option A".into()),
            },
        ],
        allow_multiple: false,
        allow_freeform: true,
        allow_comment: false,
        timeout_ms: None,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"ask""#));
    assert!(json.contains(r#""question":"Which approach?""#));
    assert!(json.contains(r#""context":"We have two options""#));
    assert!(json.contains(r#""allowFreeform":true"#));
}

#[test]
fn test_serialize_error_event() {
    let event = ServerEvent::Error {
        run_id: "run_1".into(),
        message: "Tool failed".into(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"error""#));
    assert!(json.contains(r#""message":"Tool failed""#));
}

#[test]
fn test_serialize_turn_end_event() {
    let event = ServerEvent::TurnEnd {
        run_id: "run_1".into(),
        turn: 1,
        stop_reason: "completed".into(),
        usage: Some(UsageInfo {
            input_tokens: 100,
            output_tokens: 50,
        }),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"turnEnd""#));
    assert!(json.contains(r#""stopReason":"completed""#));
    assert!(json.contains(r#""inputTokens":100"#));
    assert!(json.contains(r#""outputTokens":50"#));
}

#[test]
fn test_serialize_agent_end_event() {
    let event = ServerEvent::AgentEnd {
        run_id: "run_1".into(),
        stop_reason: "completed".into(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"agentEnd""#));
    assert!(json.contains(r#""stopReason":"completed""#));
}

#[test]
fn test_serialize_bash_output_event() {
    let event = ServerEvent::BashOutput {
        stdout: "line1\n".into(),
        stderr: String::new(),
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"bashOutput""#));
    assert!(json.contains(r#""stdout":"line1\n""#));
}

#[test]
fn test_serialize_bash_done_event() {
    let event = ServerEvent::BashDone { exit_code: 0 };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"bashDone""#));
    assert!(json.contains(r#""exitCode":0"#));
}

// -----------------------------------------------------------------------
// Type enum serde tests
// -----------------------------------------------------------------------

#[test]
fn test_thinking_level_serde() {
    for (level, expected) in &[
        (ThinkingLevel::Low, "low"),
        (ThinkingLevel::Medium, "medium"),
        (ThinkingLevel::High, "high"),
    ] {
        let json = serde_json::to_string(level).unwrap();
        assert_eq!(json, format!("\"{expected}\""));
        let back: ThinkingLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, level);
    }
}

#[test]
fn test_message_role_serde() {
    for (role, expected) in &[
        (MessageRole::System, "system"),
        (MessageRole::User, "user"),
        (MessageRole::Assistant, "assistant"),
        (MessageRole::Tool, "tool"),
    ] {
        let json = serde_json::to_string(role).unwrap();
        assert_eq!(json, format!("\"{expected}\""));
        let back: MessageRole = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, role);
    }
}

#[test]
fn test_message_serde_roundtrip() {
    let msg = Message {
        id: "msg_1".into(),
        timestamp: "2026-04-28T06:00:00Z".into(),
        role: MessageRole::User,
        content: vec![ContentBlock::Text {
            text: "Hello".into(),
        }],
        tool_call_id: None,
        tool_name: None,
        is_error: None,
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, msg.id);
    assert_eq!(back.role, msg.role);
    assert_eq!(back.content.len(), 1);
}

#[test]
fn test_message_with_tool_call_content() {
    let msg = Message {
        id: "msg_2".into(),
        timestamp: "2026-04-28T06:00:01Z".into(),
        role: MessageRole::Assistant,
        content: vec![ContentBlock::ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: serde_json::json!({"path": "src/main.rs"}),
        }],
        tool_call_id: None,
        tool_name: None,
        is_error: None,
    };
    let json = serde_json::to_string(&msg).unwrap();
    let back: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(back.role, MessageRole::Assistant);
    if let ContentBlock::ToolCall { id, name, .. } = &back.content[0] {
        assert_eq!(id, "call_1");
        assert_eq!(name, "read");
    } else {
        panic!("Expected ToolCall content block");
    }
}

#[test]
fn test_model_info_serde() {
    let model = ModelInfo {
        provider: "openai".into(),
        id: "gpt-5.5".into(),
        display_name: "GPT-5.5".into(),
        context_window: 128_000,
        supports_reasoning: false,
        supports_tools: true,
    };
    let json = serde_json::to_string(&model).unwrap();
    let back: ModelInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(back.provider, "openai");
    assert_eq!(back.id, "gpt-5.5");
    assert_eq!(back.context_window, 128_000);
}

#[test]
fn test_ask_user_response_serde() {
    let resp = AskUserResponse {
        selected: vec!["Option A".into()],
        freeform: Some("custom input".into()),
        comment: Some("a comment".into()),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let back: AskUserResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(back.selected, vec!["Option A"]);
    assert_eq!(back.freeform, Some("custom input".into()));
    assert_eq!(back.comment, Some("a comment".into()));
}

#[test]
fn test_session_header_serde() {
    let header = SessionHeader::new(
        "ses_test".into(),
        "/home/user/project".into(),
        ModelInfo {
            provider: "openai".into(),
            id: "gpt-5-codex".into(),
            display_name: "GPT-5 Codex".into(),
            context_window: 200_000,
            supports_reasoning: true,
            supports_tools: true,
        },
        ThinkingLevel::High,
    );
    let json = serde_json::to_string_pretty(&header).unwrap();
    let back: SessionHeader = serde_json::from_str(&json).unwrap();
    assert_eq!(back.session_id, "ses_test");
    assert_eq!(back.type_field, "session_header");
    assert_eq!(back.schema_version, 1);
    assert_eq!(back.cwd, "/home/user/project");
    assert_eq!(back.model.id, "gpt-5-codex");
}

// -----------------------------------------------------------------------
// UsageInfo serde
// -----------------------------------------------------------------------

#[test]
fn test_usage_info_serde() {
    let usage = UsageInfo {
        input_tokens: 150,
        output_tokens: 75,
    };
    let json = serde_json::to_string(&usage).unwrap();
    let back: UsageInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(back.input_tokens, 150);
    assert_eq!(back.output_tokens, 75);
}
