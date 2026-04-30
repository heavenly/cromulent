use super::*;
use crate::protocol::types::ContentBlock;

#[test]
fn test_message_to_llm_user_text() {
    let msg = Message {
        id: "msg_1".into(),
        timestamp: "2026-01-01T00:00:00Z".into(),
        role: MessageRole::User,
        content: vec![ContentBlock::Text {
            text: "Hello".into(),
        }],
        tool_call_id: None,
        tool_name: None,
        is_error: None,
    };

    let llm = message_to_llm(&msg).unwrap();
    assert_eq!(llm.role, "user");
    assert_eq!(llm.content.len(), 1);
}

#[test]
fn test_message_to_llm_skips_system() {
    let msg = Message {
        id: "msg_1".into(),
        timestamp: "2026-01-01T00:00:00Z".into(),
        role: MessageRole::System,
        content: vec![],
        tool_call_id: None,
        tool_name: None,
        is_error: None,
    };
    assert!(message_to_llm(&msg).is_none());
}

#[test]
fn test_new_user_message() {
    let msg = new_user_message("test");
    assert_eq!(msg.role, MessageRole::User);
    assert_eq!(msg.content.len(), 1);
}

#[test]
fn test_new_tool_result_message() {
    let msg = new_tool_result_message("call_1", "read", "file content", false, None);
    assert_eq!(msg.role, MessageRole::Tool);
    assert_eq!(msg.tool_call_id.unwrap(), "call_1");
}
