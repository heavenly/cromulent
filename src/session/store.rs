use std::path::PathBuf;

use crate::protocol::types::{Message, ModelInfo, ThinkingLevel};

/// Header metadata stored as header.json inside the session directory.
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
            schema_version: 2,
        }
    }
}

/// A fully loaded session in memory
#[derive(Debug, Clone)]
pub struct LoadedSessionState {
    pub header: SessionHeader,
    pub messages: Vec<Message>,
}

/// Manages session persistence on disk.
///
/// New sessions use a split format:
///   `sessions/<id>/header.json`   — metadata (small, updated frequently)
///   `sessions/<id>/messages.jsonl` — transcript (append-only)
///
/// Legacy sessions (schema_version 1) are stored as a single `.jsonl` file
/// and loaded transparently. New sessions always use the split format.
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

    // ------------------------------------------------------------------
    // Path helpers
    // ------------------------------------------------------------------

    /// Directory for a split-format session
    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.sessions_dir.join(session_id)
    }

    /// Header path (split format)
    pub fn header_path(&self, session_id: &str) -> PathBuf {
        self.session_dir(session_id).join("header.json")
    }

    /// Messages path (split format)
    pub fn messages_path(&self, session_id: &str) -> PathBuf {
        self.session_dir(session_id).join("messages.jsonl")
    }

    /// Legacy single-file path (v1 format: sessions/<id>.jsonl)
    pub fn legacy_path(&self, session_id: &str) -> PathBuf {
        self.sessions_dir.join(format!("{session_id}.jsonl"))
    }

    /// Check whether a session uses the legacy format
    async fn is_legacy(&self, session_id: &str) -> bool {
        tokio::fs::metadata(&self.legacy_path(session_id))
            .await
            .is_ok()
    }

    // ------------------------------------------------------------------
    // CRUD
    // ------------------------------------------------------------------

    /// Create a new session (always uses split format).
    pub async fn create_session(&self, header: &SessionHeader) -> std::io::Result<()> {
        let dir = self.session_dir(&header.session_id);
        tokio::fs::create_dir_all(&dir).await?;

        let header_path = self.header_path(&header.session_id);
        let header_json = serde_json::to_string(header).map_err(|e| std::io::Error::other(e))?;
        tokio::fs::write(&header_path, header_json).await?;

        // Create empty messages file
        let msg_path = self.messages_path(&header.session_id);
        tokio::fs::write(&msg_path, "").await?;

        Ok(())
    }

    /// Load a session from disk.
    /// Supports both legacy (single .jsonl) and split (directory) formats.
    pub async fn load_session(&self, session_id: &str) -> std::io::Result<LoadedSessionState> {
        if self.is_legacy(session_id).await {
            return self.load_legacy(session_id).await;
        }
        self.load_split(session_id).await
    }

    /// Load from legacy single-file format.
    async fn load_legacy(&self, session_id: &str) -> std::io::Result<LoadedSessionState> {
        let path = self.legacy_path(session_id);
        let content = tokio::fs::read_to_string(&path).await?;
        let lines: Vec<&str> = content.lines().collect();

        if lines.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Empty session file",
            ));
        }

        let header: SessionHeader = serde_json::from_str(lines[0])
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

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

    /// Load from split format — streams messages line-by-line.
    async fn load_split(&self, session_id: &str) -> std::io::Result<LoadedSessionState> {
        // Read header
        let header_path = self.header_path(session_id);
        let header_json = tokio::fs::read_to_string(&header_path).await?;
        let header: SessionHeader = serde_json::from_str(&header_json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        // Stream messages line-by-line (not loading entire file into memory)
        let msg_path = self.messages_path(session_id);
        let messages = self.read_messages_streaming(&msg_path).await?;

        Ok(LoadedSessionState { header, messages })
    }

    /// Read messages from a JSONL file using buffered line-by-line parsing.
    async fn read_messages_streaming(
        &self,
        path: &std::path::Path,
    ) -> std::io::Result<Vec<Message>> {
        // Check if file exists / is readable
        if !tokio::fs::metadata(path).await.is_ok() {
            return Ok(Vec::new());
        }

        let file = tokio::fs::File::open(path).await?;
        let reader = tokio::io::BufReader::new(file);
        let mut lines = tokio::io::AsyncBufReadExt::lines(reader);

        let mut messages = Vec::new();
        while let Some(line) = lines.next_line().await? {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<Message>(trimmed) {
                Ok(msg) => messages.push(msg),
                Err(e) => {
                    tracing::warn!(?e, line = %trimmed.chars().take(80).collect::<String>(), "Skipping malformed message");
                }
            }
        }

        Ok(messages)
    }

    /// Append a single message to the session transcript.
    pub async fn append_message(&self, session_id: &str, message: &Message) -> std::io::Result<()> {
        self.append_messages(session_id, std::slice::from_ref(message))
            .await
    }

    /// Append multiple messages in one write call (fewer file opens).
    pub async fn append_messages(
        &self,
        session_id: &str,
        messages: &[Message],
    ) -> std::io::Result<()> {
        if messages.is_empty() {
            return Ok(());
        }

        let path = if self.is_legacy(session_id).await {
            self.legacy_path(session_id)
        } else {
            self.messages_path(session_id)
        };

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;

        let mut buf = String::new();
        for msg in messages {
            let json = serde_json::to_string(msg).map_err(|e| std::io::Error::other(e))?;
            buf.push_str(&json);
            buf.push('\n');
        }

        tokio::io::AsyncWriteExt::write_all(&mut file, buf.as_bytes()).await
    }

    /// Update the header — now just writes a small header.json file.
    /// No longer reads or rewrites the full transcript.
    pub async fn update_header(&self, header: &SessionHeader) -> std::io::Result<()> {
        if self.is_legacy(&header.session_id).await {
            return self.update_header_legacy(header).await;
        }

        let header_path = self.header_path(&header.session_id);
        let header_json = serde_json::to_string(header).map_err(|e| std::io::Error::other(e))?;

        // Atomic write via temp file
        let tmp_path = header_path.with_extension("json.tmp");
        tokio::fs::write(&tmp_path, &header_json).await?;
        tokio::fs::rename(&tmp_path, &header_path).await
    }

    /// Legacy header update: rewrite the full single-file JSONL
    async fn update_header_legacy(&self, header: &SessionHeader) -> std::io::Result<()> {
        let path = self.legacy_path(&header.session_id);
        let header_json = serde_json::to_string(header).map_err(|e| std::io::Error::other(e))?;

        let existing = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        let rest: Vec<&str> = existing.lines().skip(1).collect();
        let rest_content = rest.join("\n");

        let tmp_path = path.with_extension("jsonl.tmp");
        let content = if rest_content.is_empty() {
            format!("{header_json}\n")
        } else {
            format!("{header_json}\n{rest_content}\n")
        };
        tokio::fs::write(&tmp_path, &content).await?;
        tokio::fs::rename(&tmp_path, &path).await
    }

    /// List all session IDs with their last-updated timestamps.
    /// Reads header.json for split sessions, first line for legacy sessions.
    pub async fn list_session_headers(&self) -> std::io::Result<Vec<(String, String)>> {
        let mut entries = tokio::fs::read_dir(&self.sessions_dir).await?;
        let mut sessions = Vec::new();

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            // Split format: directory with header.json
            if path.is_dir() {
                let header_path = path.join("header.json");
                if let Ok(content) = tokio::fs::read_to_string(&header_path).await {
                    let updated = serde_json::from_str::<serde_json::Value>(&content)
                        .ok()
                        .and_then(|v| v.get("updated").cloned())
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_else(|| "unknown".to_string());

                    if let Some(dir_name) = path.file_name() {
                        sessions.push((dir_name.to_string_lossy().to_string(), updated));
                    }
                }
                continue;
            }

            // Legacy format: single .jsonl file
            if path.extension().is_some_and(|ext| ext == "jsonl") {
                if let Some(stem) = path.file_stem() {
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

        sessions.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(sessions)
    }

    /// Delete a session.
    pub async fn delete_session(&self, session_id: &str) -> std::io::Result<()> {
        // Try split format
        let dir = self.session_dir(session_id);
        if tokio::fs::metadata(&dir).await.is_ok() {
            return tokio::fs::remove_dir_all(&dir).await;
        }
        // Try legacy format
        let legacy = self.legacy_path(session_id);
        if tokio::fs::metadata(&legacy).await.is_ok() {
            return tokio::fs::remove_file(&legacy).await;
        }
        Ok(())
    }

    /// Create a new session by forking from an existing one.
    pub async fn fork_session(
        &self,
        source_session_id: &str,
        up_to_entry_id: &str,
        new_header: &SessionHeader,
    ) -> std::io::Result<LoadedSessionState> {
        let source = self.load_session(source_session_id).await?;

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
        self.append_messages(&new_header.session_id, &forked_messages)
            .await?;

        Ok(LoadedSessionState {
            header: new_header.clone(),
            messages: forked_messages,
        })
    }
}
