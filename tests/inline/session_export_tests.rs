use super::*;
use crate::protocol::types::{ContentBlock, MessageRole, ModelInfo, ThinkingLevel};
use crate::util::time::now_iso;

fn sample_session() -> LoadedSessionState {
    let header = SessionHeader::new(
        "ses_test123".into(),
        "/tmp".into(),
        ModelInfo {
            provider: "openai".into(),
            id: "gpt-5.5".into(),
            display_name: "GPT-5.5".into(),
            context_window: 128_000,
            supports_reasoning: false,
            supports_tools: true,
        },
        ThinkingLevel::Medium,
    );
    let messages = vec![
        Message {
            id: "msg_1".into(),
            timestamp: now_iso(),
            role: MessageRole::User,
            content: vec![ContentBlock::Text {
                text: "Hello".into(),
            }],
            tool_call_id: None,
            tool_name: None,
            is_error: None,
        },
        Message {
            id: "msg_2".into(),
            timestamp: now_iso(),
            role: MessageRole::Assistant,
            content: vec![ContentBlock::Text {
                text: "Hi there".into(),
            }],
            tool_call_id: None,
            tool_name: None,
            is_error: None,
        },
    ];
    LoadedSessionState { header, messages }
}

#[test]
fn test_session_export_from_loaded() {
    let session = sample_session();
    let export = SessionExport::from(&session);
    assert_eq!(export.schema_version, 1);
    assert_eq!(export.messages.len(), 2);
    assert_eq!(export.header.session_id, "ses_test123");
}
