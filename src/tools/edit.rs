use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{ContentBlock, ToolContext, ToolDefinition, ToolResult};
use crate::tools::registry::Tool;
use crate::tools::ToolError;

/// Edits a file by performing exact-text replacements.
/// Each edit requires a unique, non-overlapping `oldText` occurrence in the original file.
/// All edits are matched against the original content (not incrementally).
pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "edit".to_string(),
            description: "Edit a file using exact text replacements. Each edit must have a unique, non-overlapping 'oldText' that appears exactly once in the original file. Edits are matched against the original content, not incrementally.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to edit (relative or absolute)"
                    },
                    "edits": {
                        "type": "array",
                        "description": "One or more targeted replacements. Each edit is matched against the original file, not incrementally.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "oldText": {
                                    "type": "string",
                                    "description": "Exact text to find — must appear exactly once in the file"
                                },
                                "newText": {
                                    "type": "string",
                                    "description": "Replacement text"
                                }
                            },
                            "required": ["oldText", "newText"]
                        },
                        "minItems": 1
                    }
                },
                "required": ["path", "edits"]
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

        let edits = arguments
            .get("edits")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ToolError::InvalidArguments("Missing or invalid 'edits' array".into()))?;

        if edits.is_empty() {
            return Err(ToolError::InvalidArguments("'edits' array must not be empty".into()));
        }

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

        // Parse edits, find spans in original content, and validate uniqueness
        let mut spans: Vec<(usize, usize, String, String)> = Vec::new(); // (start, end, oldText, newText)

        for (i, edit) in edits.iter().enumerate() {
            let old_text = edit
                .get("oldText")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ToolError::InvalidArguments(format!("edits[{i}]: missing 'oldText'"))
                })?;
            let new_text = edit
                .get("newText")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ToolError::InvalidArguments(format!("edits[{i}]: missing 'newText'"))
                })?;

            // Find all occurrences
            let mut start = 0;
            let mut occurrences = Vec::new();
            while let Some(pos) = content[start..].find(old_text) {
                let abs_pos = start + pos;
                occurrences.push(abs_pos);
                start = abs_pos + old_text.len();
            }

            if occurrences.is_empty() {
                return Err(ToolError::EditFailed(format!(
                    "edits[{i}]: oldText {:?} not found in file",
                    old_text
                )));
            }
            if occurrences.len() > 1 {
                return Err(ToolError::EditFailed(format!(
                    "edits[{i}]: oldText {:?} found {} times (must be unique)",
                    old_text,
                    occurrences.len()
                )));
            }

            let pos = occurrences[0];
            spans.push((pos, pos + old_text.len(), old_text.to_string(), new_text.to_string()));
        }

        // Verify no overlapping spans
        spans.sort_by_key(|s| s.0);
        for i in 1..spans.len() {
            if spans[i].0 < spans[i - 1].1 {
                // Overlap detected
                let a = &spans[i - 1];
                let b = &spans[i];
                return Err(ToolError::EditFailed(format!(
                    "Edits overlap: oldText {:?} (bytes {}-{}) and oldText {:?} (bytes {}-{})",
                    a.2, a.0, a.1, b.2, b.0, b.1
                )));
            }
        }

        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        // Apply edits in reverse order (preserves byte positions)
        let mut result = content.to_string();
        for (start, end, _old, new) in spans.iter().rev() {
            result.replace_range(*start..*end, new);
        }

        tokio::fs::write(&canonical, &result).await?;

        let replacements: Vec<serde_json::Value> = edits
            .iter()
            .map(|e| serde_json::json!({
                "oldText": e.get("oldText").and_then(|v| v.as_str()),
                "newText": e.get("newText").and_then(|v| v.as_str()),
            }))
            .collect();

        Ok(ToolResult {
            content: vec![ContentBlock::Text {
                text: format!("Applied {} edit(s) to '{}'", edits.len(), canonical.display()),
            }],
            is_error: false,
            metadata: Some(serde_json::json!({
                "path": canonical.to_string_lossy(),
                "edits": replacements,
            })),
        })
    }
}
