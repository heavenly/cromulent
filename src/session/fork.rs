use crate::protocol::types::{Message, ModelInfo, ThinkingLevel};
use crate::session::store::{LoadedSessionState, SessionHeader};
use crate::util::time::now_iso;

/// Options for forking a session.
#[derive(Debug, Clone)]
pub struct ForkOptions {
    /// If set, override the model info in the forked header.
    pub model: Option<ModelInfo>,
    /// If set, override the thinking level in the forked header.
    pub thinking_level: Option<ThinkingLevel>,
    /// If set, override the cwd in the forked header.
    pub cwd: Option<String>,
}

impl Default for ForkOptions {
    fn default() -> Self {
        Self {
            model: None,
            thinking_level: None,
            cwd: None,
        }
    }
}

/// Fork a loaded session up to (and including) the message with the given entry ID.
///
/// Returns a new `LoadedSessionState` with a fresh session ID, current timestamps,
/// and only the messages up to the specified entry.
///
/// # Errors
///
/// Returns `Err` if `entry_id` is not found in the session's messages.
pub fn fork_session_helper(
    session: &LoadedSessionState,
    entry_id: &str,
    options: ForkOptions,
) -> Result<LoadedSessionState, ForkError> {
    // Find the entry index
    let idx = session
        .messages
        .iter()
        .position(|msg| msg.id == entry_id)
        .ok_or_else(|| ForkError::EntryNotFound(entry_id.to_string()))?;

    let forked_messages: Vec<Message> = session.messages[..=idx].to_vec();
    let now = now_iso();

    let header = SessionHeader {
        type_field: "session_header".to_string(),
        session_id: crate::util::ids::generate_session_id(),
        created: now.clone(),
        updated: now,
        cwd: options
            .cwd
            .unwrap_or_else(|| session.header.cwd.clone()),
        model: options.model.unwrap_or_else(|| session.header.model.clone()),
        thinking_level: options
            .thinking_level
            .clone()
            .unwrap_or_else(|| session.header.thinking_level.clone()),
        schema_version: session.header.schema_version,
    };

    Ok(LoadedSessionState {
        header,
        messages: forked_messages,
    })
}

/// Errors that can occur when forking a session.
#[derive(Debug, thiserror::Error)]
pub enum ForkError {
    /// The specified entry ID was not found in the session's messages.
    #[error("entry not found: {0}")]
    EntryNotFound(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::types::{ContentBlock, MessageRole, ModelInfo, ThinkingLevel};
    use crate::util::time::now_iso;

    fn sample_session() -> LoadedSessionState {
        let header = SessionHeader::new(
            "ses_original".into(),
            "/proj".into(),
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
                    text: "First".into(),
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
                    text: "Second".into(),
                }],
                tool_call_id: None,
                tool_name: None,
                is_error: None,
            },
            Message {
                id: "msg_3".into(),
                timestamp: now_iso(),
                role: MessageRole::User,
                content: vec![ContentBlock::Text {
                    text: "Third".into(),
                }],
                tool_call_id: None,
                tool_name: None,
                is_error: None,
            },
        ];
        LoadedSessionState { header, messages }
    }

    #[test]
    fn test_fork_up_to_entry_id() {
        let session = sample_session();
        let forked = fork_session_helper(&session, "msg_2", ForkOptions::default()).unwrap();

        // Should have messages up to msg_2 (0..=1)
        assert_eq!(forked.messages.len(), 2);
        assert_eq!(forked.messages[0].id, "msg_1");
        assert_eq!(forked.messages[1].id, "msg_2");

        // Should have a new session ID
        assert_ne!(forked.header.session_id, "ses_original");
        assert!(forked.header.session_id.starts_with("ses_"));

        // Cwd and model should be inherited
        assert_eq!(forked.header.cwd, "/proj");
        assert_eq!(forked.header.model.id, "gpt-5.5");
    }

    #[test]
    fn test_fork_with_options() {
        let session = sample_session();
        let opts = ForkOptions {
            model: Some(ModelInfo {
                provider: "anthropic".into(),
                id: "claude-4".into(),
                display_name: "Claude 4".into(),
                context_window: 200_000,
                supports_reasoning: true,
                supports_tools: true,
            }),
            cwd: Some("/new/proj".into()),
            ..Default::default()
        };

        let forked = fork_session_helper(&session, "msg_3", opts).unwrap();
        assert_eq!(forked.header.model.provider, "anthropic");
        assert_eq!(forked.header.model.id, "claude-4");
        assert_eq!(forked.header.cwd, "/new/proj");
        assert_eq!(forked.messages.len(), 3);
    }

    #[test]
    fn test_fork_entry_not_found() {
        let session = sample_session();
        let result = fork_session_helper(&session, "msg_nonexistent", ForkOptions::default());
        assert!(result.is_err());
        match result {
            Err(ForkError::EntryNotFound(id)) => assert_eq!(id, "msg_nonexistent"),
            _ => panic!("expected EntryNotFound"),
        }
    }
}
