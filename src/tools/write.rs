use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{ContentBlock, ToolContext, ToolDefinition, ToolResult};
use crate::tools::registry::Tool;
use crate::tools::ToolError;

/// Writes content to a file, automatically creating parent directories.
pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write".to_string(),
            description: "Write content to a file. Creates the file and all parent directories as needed. Overwrites if the file already exists.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to write (relative or absolute)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
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

        let content = arguments
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("Missing required 'content' argument".into()))?;

        let abs_path = ctx.cwd.join(path_str);

        // Create parent directories
        if let Some(parent) = abs_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        tokio::fs::write(&abs_path, content).await?;

        let file_size = content.len();

        Ok(ToolResult {
            content: vec![ContentBlock::Text {
                text: format!("Wrote {} bytes to '{}'", file_size, abs_path.display()),
            }],
            is_error: false,
            metadata: Some(serde_json::json!({
                "path": abs_path.to_string_lossy(),
                "bytes": file_size,
            })),
        })
    }
}
