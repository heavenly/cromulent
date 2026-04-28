use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{ContentBlock, ToolContext, ToolDefinition, ToolResult};
use crate::tools::registry::Tool;
use crate::tools::ToolError;

/// Reads a text file, optionally limiting to a range of lines.
/// Relative paths are resolved against `ToolContext::cwd`.
pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read".to_string(),
            description: "Read a text file. Optionally specify offset (1-indexed start line) and limit (max lines). Relative paths are relative to the current working directory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to read (relative or absolute)"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Starting line number (1-indexed, default: 1)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of lines to return (default: no limit)"
                    }
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
            .ok_or_else(|| ToolError::InvalidArguments("Missing required 'path' argument".into()))?;

        let abs_path = ctx.cwd.join(path_str);
        let canonical = std::fs::canonicalize(&abs_path)
            .map_err(|e| ToolError::Other(format!("Cannot resolve path '{}': {e}", abs_path.display())))?;

        if !canonical.is_file() {
            return Err(ToolError::NotFound(format!("Not a file: '{}'", canonical.display())));
        }

        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let content = tokio::fs::read_to_string(&canonical).await?;

        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let offset = arguments
            .get("offset")
            .and_then(|v| v.as_i64())
            .unwrap_or(1)
            .max(1) as usize;

        let limit = arguments.get("limit").and_then(|v| v.as_i64());

        let start = (offset - 1).min(total_lines);
        let end = match limit {
            Some(l) => (start + l as usize).min(total_lines),
            None => total_lines,
        };

        let excerpt = lines[start..end].join("\n");
        let line_count = end - start;

        let mut text = format!("File: {}\n", canonical.display());
        if line_count < total_lines {
            text.push_str(&format!(
                "Showing lines {}-{} of {}\n\n",
                start + 1,
                end,
                total_lines
            ));
        } else {
            text.push_str(&format!("Total lines: {}\n\n", total_lines));
        }
        text.push_str(&excerpt);

        Ok(ToolResult {
            content: vec![ContentBlock::Text { text }],
            is_error: false,
            metadata: Some(serde_json::json!({
                "path": canonical.to_string_lossy(),
                "totalLines": total_lines,
                "startLine": start + 1,
                "endLine": end,
                "lineCount": line_count,
            })),
        })
    }
}
