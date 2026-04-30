use async_trait::async_trait;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{ContentBlock, ToolContext, ToolDefinition, ToolResult};
use crate::tools::registry::Tool;
use crate::tools::ToolError;

/// Searches for files by glob pattern using the `ignore` crate.
/// Respects `.gitignore` rules automatically and uses `globset` for proper
/// glob matching. Scales well on large repositories.
pub struct FindTool;

#[async_trait]
impl Tool for FindTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "find".to_string(),
            description: "Search for files by glob pattern (e.g. '*.rs', 'src/**/*.ts'). Relative paths are resolved against cwd. Returns matching file paths. Respects .gitignore.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern to match files, e.g. '*.rs' or 'src/**/*.ts'"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in (default: cwd)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (default: 1000)"
                    }
                },
                "required": ["pattern"]
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

        let pattern_str = arguments
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidArguments("Missing required 'pattern' argument".into())
            })?;

        let max_results = arguments
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(1000)
            .max(1) as usize;

        let search_path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| ctx.cwd.join(p))
            .unwrap_or_else(|| ctx.cwd.clone());

        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        // Build a GlobSet from the pattern (handles braces like {rs,toml})
        let glob_set = build_glob_set(pattern_str)
            .map_err(|e| ToolError::InvalidArguments(format!("Invalid glob pattern: {e}")))?;

        // Use ignore::WalkBuilder for gitignore-aware traversal
        let walker = WalkBuilder::new(&search_path)
            .follow_links(false)
            .hidden(false) // ignore crate handles .gitignore, not dotfile prefixes
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .require_git(false) // don't require the dir to be a git repo
            .sort_by_file_path(std::path::Path::cmp)
            .build();

        let mut results: Vec<String> = Vec::new();
        let mut count = 0;

        for entry in walker {
            if cancel.is_cancelled() {
                return Err(ToolError::Cancelled);
            }

            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    // Skip permission errors etc.
                    tracing::debug!(error = %err, "Walk entry error");
                    continue;
                }
            };

            // Only match files (not directories)
            if entry.file_type().map_or(true, |ft| !ft.is_file()) {
                continue;
            }

            let rel_path = entry
                .path()
                .strip_prefix(&search_path)
                .unwrap_or(entry.path());

            if glob_set.is_match(rel_path) {
                results.push(rel_path.to_string_lossy().to_string());
                count += 1;
                if count >= max_results {
                    break;
                }
            }
        }

        if results.is_empty() {
            return Ok(ToolResult {
                content: vec![ContentBlock::Text {
                    text: format!("No files found matching pattern: {pattern_str}"),
                }],
                is_error: false,
                metadata: Some(serde_json::json!({
                    "pattern": pattern_str,
                    "results": 0,
                })),
            });
        }

        let mut output = format!(
            "Found {} file(s) matching pattern: {pattern_str}\n\n",
            count
        );
        for (i, path) in results.iter().enumerate() {
            output.push_str(&format!("{}. {}\n", i + 1, path));
        }

        if count >= max_results {
            output.push_str(&format!("\n[Reached limit of {} results]", max_results));
        }

        Ok(ToolResult {
            content: vec![ContentBlock::Text { text: output }],
            is_error: false,
            metadata: Some(serde_json::json!({
                "pattern": pattern_str,
                "results": count,
                "limit": max_results,
            })),
        })
    }
}

/// Build a GlobSet from a glob pattern string.
/// Supports standard glob syntax including `{a,b}` brace expansion.
fn build_glob_set(pattern: &str) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    // Split on whitespace to allow multiple patterns (convenience)
    for part in pattern.split_whitespace() {
        if part.is_empty() {
            continue;
        }
        let glob = Glob::new(part).map_err(|e| format!("{e}"))?;
        builder.add(glob);
    }
    builder.build().map_err(|e| format!("{e}"))
}
