use crate::protocol::types::{Message, ModelInfo, ThinkingLevel};
use crate::session::store::{LoadedSessionState, SessionHeader};
use crate::util::time::now_iso;

/// Options for forking a session.
#[derive(Debug, Clone, Default)]
pub struct ForkOptions {
    /// If set, override the model info in the forked header.
    pub model: Option<ModelInfo>,
    /// If set, override the thinking level in the forked header.
    pub thinking_level: Option<ThinkingLevel>,
    /// If set, override the cwd in the forked header.
    pub cwd: Option<String>,
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
        cwd: options.cwd.unwrap_or_else(|| session.header.cwd.clone()),
        model: options
            .model
            .unwrap_or_else(|| session.header.model.clone()),
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
#[path = "../../tests/inline/session_fork_tests.rs"]
mod tests;
