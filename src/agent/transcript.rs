use crate::protocol::types::{
    ContentBlock, LlmContentBlock, LlmMessage, Message, MessageRole,
};
use crate::util::ids::generate_message_id;
use crate::util::time::now_iso;

// ---------------------------------------------------------------------------
// Message -> LlmMessage conversion
// ---------------------------------------------------------------------------

/// Convert a [`Message`] (internal transcript format) to a [`LlmMessage`]
/// (provider request format).
///
/// * `MessageRole::System` is skipped (system prompt is injected separately).
/// * Tool messages carry a single `ToolResult` content block.
/// * Text and tool-call content blocks are mapped directly.
pub fn message_to_llm(message: &Message) -> Option<LlmMessage> {
    if message.role == MessageRole::System {
        return None;
    }

    let role = match message.role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
        MessageRole::System => return None,
    };

    let content: Vec<LlmContentBlock> = message
        .content
        .iter()
        .filter_map(content_block_to_llm)
        .collect();

    if content.is_empty() {
        return None;
    }

    Some(LlmMessage {
        role: role.to_string(),
        content,
    })
}

/// Convert a [`ContentBlock`] to a [`LlmContentBlock`].
fn content_block_to_llm(block: &ContentBlock) -> Option<LlmContentBlock> {
    match block {
        ContentBlock::Text { text } => Some(LlmContentBlock::Text {
            text: text.clone(),
        }),
        ContentBlock::ToolCall {
            id,
            name,
            arguments,
        } => Some(LlmContentBlock::ToolCall {
            id: id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
        }),
        ContentBlock::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => {
            // Flatten tool result content to a single text string
            let text: String = content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");

            Some(LlmContentBlock::ToolResult {
                tool_call_id: tool_call_id.clone(),
                content: text,
                is_error: *is_error,
            })
        }
    }
}

/// Bulk convert a slice of messages to LlmMessage, filtering out system messages.
pub fn messages_to_llm(messages: &[Message]) -> Vec<LlmMessage> {
    messages
        .iter()
        .filter_map(message_to_llm)
        .collect()
}

// ---------------------------------------------------------------------------
// Builder helpers
// ---------------------------------------------------------------------------

/// Create a new user message from text.
pub fn new_user_message(text: impl Into<String>) -> Message {
    Message {
        id: generate_message_id(),
        timestamp: now_iso(),
        role: MessageRole::User,
        content: vec![ContentBlock::Text {
            text: text.into(),
        }],
        tool_call_id: None,
        tool_name: None,
        is_error: None,
    }
}

/// Create a new assistant message containing text.
pub fn new_assistant_text_message(text: impl Into<String>) -> Message {
    Message {
        id: generate_message_id(),
        timestamp: now_iso(),
        role: MessageRole::Assistant,
        content: vec![ContentBlock::Text {
            text: text.into(),
        }],
        tool_call_id: None,
        tool_name: None,
        is_error: None,
    }
}

/// Create a new assistant message with one or more tool calls.
pub fn new_assistant_tool_call_message(tool_calls: Vec<LlmContentBlock>) -> Message {
    let content: Vec<ContentBlock> = tool_calls
        .into_iter()
        .filter_map(|tc| match tc {
            LlmContentBlock::ToolCall { id, name, arguments } => {
                Some(ContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                })
            }
            _ => None,
        })
        .collect();

    Message {
        id: generate_message_id(),
        timestamp: now_iso(),
        role: MessageRole::Assistant,
        content,
        tool_call_id: None,
        tool_name: None,
        is_error: None,
    }
}

/// Create a new assistant message combining text and tool calls
/// (for providers that emit both in one response).
pub fn new_assistant_message(
    text: Option<String>,
    tool_calls: Vec<LlmContentBlock>,
) -> Message {
    let mut content = Vec::new();

    if let Some(t) = text {
        content.push(ContentBlock::Text { text: t });
    }

    for tc in tool_calls {
        if let LlmContentBlock::ToolCall { id, name, arguments } = tc {
            content.push(ContentBlock::ToolCall {
                id,
                name,
                arguments,
            });
        }
    }

    Message {
        id: generate_message_id(),
        timestamp: now_iso(),
        role: MessageRole::Assistant,
        content,
        tool_call_id: None,
        tool_name: None,
        is_error: None,
    }
}

/// Create a new tool-result message.
pub fn new_tool_result_message(
    tool_call_id: impl Into<String>,
    tool_name: impl Into<String>,
    content_text: impl Into<String>,
    is_error: bool,
) -> Message {
    let tool_call_id: String = tool_call_id.into();
    let tool_name: String = tool_name.into();
    let content_text: String = content_text.into();
    Message {
        id: generate_message_id(),
        timestamp: now_iso(),
        role: MessageRole::Tool,
        content: vec![ContentBlock::ToolResult {
            tool_call_id: tool_call_id.clone(),
            content: vec![ContentBlock::Text {
                text: content_text,
            }],
            is_error,
        }],
        tool_call_id: Some(tool_call_id),
        tool_name: Some(tool_name),
        is_error: Some(is_error),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
        let msg = new_tool_result_message("call_1", "read", "file content", false);
        assert_eq!(msg.role, MessageRole::Tool);
        assert_eq!(msg.tool_call_id.unwrap(), "call_1");
    }
}
