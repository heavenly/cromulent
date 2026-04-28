use std::path::Path;

use crate::protocol::types::Message;
use crate::session::store::{LoadedSessionState, SessionHeader};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Portable JSON export format
// ---------------------------------------------------------------------------

/// Portable session export in a single JSON object.
/// Preferred over raw JSONL for interchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionExport {
    pub schema_version: u32,
    pub header: SessionHeader,
    pub messages: Vec<Message>,
}

impl From<&LoadedSessionState> for SessionExport {
    fn from(state: &LoadedSessionState) -> Self {
        Self {
            schema_version: 1,
            header: state.header.clone(),
            messages: state.messages.clone(),
        }
    }
}

/// Export a loaded session to a portable JSON file.
///
/// The output file contains a single JSON object:
/// ```json
/// {
///   "schemaVersion": 1,
///   "header": { ... },
///   "messages": [ ... ]
/// }
/// ```
pub async fn export_session(
    path: impl AsRef<Path>,
    session: &LoadedSessionState,
) -> std::io::Result<()> {
    let export = SessionExport::from(session);
    let json = serde_json::to_string_pretty(&export)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    tokio::fs::write(path.as_ref(), &json).await
}

/// Load a session from a portable JSON export file.
pub async fn load_export(
    path: impl AsRef<Path>,
) -> std::io::Result<LoadedSessionState> {
    let content = tokio::fs::read_to_string(path.as_ref()).await?;
    let export: SessionExport = serde_json::from_str(&content)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    if export.schema_version != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Unsupported export schema version: {}", export.schema_version),
        ));
    }

    Ok(LoadedSessionState {
        header: export.header,
        messages: export.messages,
    })
}

#[cfg(test)]
mod tests {
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

    #[tokio::test]
    async fn test_export_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");

        let original = sample_session();
        export_session(&path, &original).await.unwrap();

        let loaded = load_export(&path).await.unwrap();
        assert_eq!(loaded.header.session_id, original.header.session_id);
        assert_eq!(loaded.messages.len(), original.messages.len());
        assert_eq!(loaded.messages[0].id, original.messages[0].id);
        assert_eq!(loaded.messages[1].id, original.messages[1].id);
    }

    #[test]
    fn test_session_export_from_loaded() {
        let session = sample_session();
        let export = SessionExport::from(&session);
        assert_eq!(export.schema_version, 1);
        assert_eq!(export.messages.len(), 2);
        assert_eq!(export.header.session_id, "ses_test123");
    }
}
