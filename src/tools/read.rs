use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{ContentBlock, ToolContext, ToolDefinition, ToolResult};
use crate::tools::hashline::file_kind::{load_file, LoadedFile};
use crate::tools::hashline::read::format_read_preview;
use crate::tools::registry::Tool;
use crate::tools::ToolError;

/// Reads a UTF-8 text file as hash-anchored lines (`LINE#HASH:content`).
/// Relative paths are resolved against `ToolContext::cwd`.
pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read".to_string(),
            description: "Read a UTF-8 text file as hash-anchored lines in the form LINE#HASH:content. Use the returned anchors with hashline_edit. Optionally specify offset (1-indexed start line) and limit (max lines). Rejects directories and binary files.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to read (relative or absolute)" },
                    "offset": { "type": "integer", "description": "Starting line number (1-indexed, default: 1)", "minimum": 1 },
                    "limit": { "type": "integer", "description": "Maximum number of lines to return", "minimum": 1 }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolContext,
        arguments: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<ToolResult, ToolError> {
        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let path_str = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("Missing required 'path' argument".into())
            })?;

        let offset = arguments
            .get("offset")
            .and_then(|v| v.as_i64())
            .unwrap_or(1);
        if offset < 1 {
            return Err(ToolError::InvalidArguments(
                "Read request field \"offset\" must be a positive integer.".into(),
            ));
        }
        let limit = arguments.get("limit").and_then(|v| v.as_i64());
        if matches!(limit, Some(l) if l < 1) {
            return Err(ToolError::InvalidArguments(
                "Read request field \"limit\" must be a positive integer.".into(),
            ));
        }

        let abs_path = ctx.cwd.join(path_str);
        let canonical = std::fs::canonicalize(&abs_path).map_err(|e| {
            ToolError::Other(format!("Cannot resolve path '{}': {e}", abs_path.display()))
        })?;

        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let loaded = load_file(&canonical).await?;
        let text = match loaded {
            LoadedFile::Text(t) => t,
            LoadedFile::Directory => return Err(ToolError::EditFailed(format!("[E_DIRECTORY] Path is a directory: {path_str}. Use find/ls-style tools to inspect directories."))),
            LoadedFile::Image(mime) => return Err(ToolError::EditFailed(format!("[E_BINARY_FILE] Path is an image file ({mime}); hashline read only supports UTF-8 text files."))),
            LoadedFile::Binary(desc) => return Err(ToolError::EditFailed(format!("[E_BINARY_FILE] Path is a binary file: {path_str} ({desc}). Hashline read only supports UTF-8 text files."))),
        };

        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let preview = format_read_preview(&text, offset as usize, limit.map(|l| l as usize))
            .map_err(ToolError::InvalidArguments)?;

        Ok(ToolResult {
            content: vec![ContentBlock::Text { text: preview.text }],
            is_error: false,
            metadata: Some(serde_json::json!({
                "path": canonical.to_string_lossy(),
                "fileKind": "text",
                "offset": preview.start,
                "lineCountReturned": preview.returned,
                "totalLines": preview.total_lines,
                "truncated": preview.truncated,
                "nextOffset": preview.next_offset,
                "startLine": preview.start,
                "endLine": preview.end,
                "fileHash": preview.file_hash,
            })),
        })
    }
}
