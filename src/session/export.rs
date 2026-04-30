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
    let json = serde_json::to_string_pretty(&export).map_err(|e| std::io::Error::other(e))?;
    tokio::fs::write(path.as_ref(), &json).await
}

#[cfg(test)]
#[path = "../../tests/inline/session_export_tests.rs"]
mod tests;
