use std::path::PathBuf;

use crate::protocol::types::{Message, ModelInfo, ThinkingLevel};

/// Header metadata stored as the first line of a session file
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHeader {
    #[serde(rename = "type")]
    pub type_field: String,
    pub session_id: String,
    pub created: String,
    pub updated: String,
    pub cwd: String,
    pub model: ModelInfo,
    pub thinking_level: ThinkingLevel,
    pub schema_version: u32,
}

impl SessionHeader {
    pub fn new(
        session_id: String,
        cwd: String,
        model: ModelInfo,
        thinking_level: ThinkingLevel,
    ) -> Self {
        let now = crate::util::time::now_iso();
        Self {
            type_field: "session_header".to_string(),
            session_id,
            created: now.clone(),
            updated: now,
            cwd,
            model,
            thinking_level,
            schema_version: 1,
        }
    }
}

/// A fully loaded session in memory
#[derive(Debug, Clone)]
pub struct LoadedSessionState {
    pub header: SessionHeader,
    pub messages: Vec<Message>,
}

/// Manages session persistence on disk
#[derive(Clone)]
pub struct SessionStore {
    sessions_dir: PathBuf,
}

impl SessionStore {
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
    }

    /// Ensure the sessions directory exists
    pub async fn ensure_dir(&self) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.sessions_dir).await
    }

    /// Path to the session file for a given session ID
    pub fn session_path(&self, session_id: &str) -> PathBuf {
        self.sessions_dir.join(format!("{}.jsonl", session_id))
    }

    /// Create a new session file with a header
    pub async fn create_session(&self, header: &SessionHeader) -> std::io::Result<()> {
        self.ensure_dir().await?;
        let path = self.session_path(&header.session_id);
        let header_json = serde_json::to_string(header).map_err(|e| std::io::Error::other(e))?;
        tokio::fs::write(&path, format!("{header_json}\n")).await
    }

    /// Load a session from disk, returning header and messages
    pub async fn load_session(&self, session_id: &str) -> std::io::Result<LoadedSessionState> {
        let path = self.session_path(session_id);
        let content = tokio::fs::read_to_string(&path).await?;
        let lines: Vec<&str> = content.lines().collect();

        if lines.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Empty session file",
            ));
        }

        // First line is the header
        let header: SessionHeader = serde_json::from_str(lines[0])
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        // Validate schema version
        if header.schema_version != 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unsupported schema version: {}", header.schema_version),
            ));
        }

        // Remaining lines are messages
        let mut messages = Vec::new();
        for line in &lines[1..] {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Message>(line) {
                Ok(msg) => messages.push(msg),
                Err(e) => {
                    tracing::warn!("Skipping malformed message: {e}");
                }
            }
        }

        Ok(LoadedSessionState { header, messages })
    }

    /// Append a message to the session file
    pub async fn append_message(&self, session_id: &str, message: &Message) -> std::io::Result<()> {
        let path = self.session_path(session_id);
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        let msg_json = serde_json::to_string(message).map_err(|e| std::io::Error::other(e))?;
        tokio::io::AsyncWriteExt::write_all(&mut file, format!("{msg_json}\n").as_bytes()).await
    }

    /// Update the header in a session file atomically
    pub async fn update_header(&self, header: &SessionHeader) -> std::io::Result<()> {
        let path = self.session_path(&header.session_id);
        let header_json = serde_json::to_string(header).map_err(|e| std::io::Error::other(e))?;

        // Read existing content (messages after header)
        let existing = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        let rest: Vec<&str> = existing.lines().skip(1).collect();
        let rest_content = rest.join("\n");

        // Write temp file then rename
        let tmp_path = path.with_extension("jsonl.tmp");
        let content = if rest_content.is_empty() {
            format!("{header_json}\n")
        } else {
            format!("{header_json}\n{rest_content}\n")
        };
        tokio::fs::write(&tmp_path, &content).await?;
        tokio::fs::rename(&tmp_path, &path).await
    }

    /// List all session IDs with their last-updated timestamps
    pub async fn list_session_headers(&self) -> std::io::Result<Vec<(String, String)>> {
        let mut entries = tokio::fs::read_dir(&self.sessions_dir).await?;
        let mut sessions = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "jsonl") {
                if let Some(stem) = path.file_stem() {
                    // Read first line to get the header's updated timestamp
                    if let Ok(content) = tokio::fs::read_to_string(&path).await {
                        if let Some(first_line) = content.lines().next() {
                            let updated = serde_json::from_str::<serde_json::Value>(first_line)
                                .ok()
                                .and_then(|v| v.get("updated").cloned())
                                .and_then(|v| v.as_str().map(String::from))
                                .unwrap_or_else(|| "unknown".to_string());
                            sessions.push((stem.to_string_lossy().to_string(), updated));
                        }
                    }
                }
            }
        }
        // Sort by updated descending (most recent first)
        sessions.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(sessions)
    }

    /// Delete a session file from disk.
    pub async fn delete_session(&self, session_id: &str) -> std::io::Result<()> {
        let path = self.session_path(session_id);
        tokio::fs::remove_file(&path).await
    }

    /// Create a new session by forking from an existing one
    pub async fn fork_session(
        &self,
        source_session_id: &str,
        up_to_entry_id: &str,
        new_header: &SessionHeader,
    ) -> std::io::Result<LoadedSessionState> {
        let source = self.load_session(source_session_id).await?;

        // Find the entry and copy messages up to and including it
        let mut idx = None;
        for (i, msg) in source.messages.iter().enumerate() {
            if msg.id == up_to_entry_id {
                idx = Some(i);
                break;
            }
        }

        let idx = idx.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Entry not found: {up_to_entry_id}"),
            )
        })?;

        let forked_messages: Vec<Message> = source.messages[..=idx].to_vec();

        // Write the new session
        self.create_session(new_header).await?;
        for msg in &forked_messages {
            self.append_message(&new_header.session_id, msg).await?;
        }

        Ok(LoadedSessionState {
            header: new_header.clone(),
            messages: forked_messages,
        })
    }
}
