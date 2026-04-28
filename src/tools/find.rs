use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use regex::Regex;

use crate::protocol::types::{ContentBlock, ToolContext, ToolDefinition, ToolResult};
use crate::tools::registry::Tool;
use crate::tools::ToolError;

/// Searches for files by glob pattern using walkdir.
pub struct FindTool;

#[async_trait]
impl Tool for FindTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "find".to_string(),
            description: "Search for files by glob pattern (e.g. '*.rs', 'src/**/*.ts'). Relative paths are resolved against cwd. Returns matching file paths.".to_string(),
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
            .ok_or_else(|| ToolError::InvalidArguments("Missing required 'pattern' argument".into()))?;

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

        // Convert glob to regex
        let re = glob_to_regex(pattern_str);

        let walk = walkdir::WalkDir::new(&search_path)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let fname = e.file_name().to_string_lossy();
                !fname.starts_with('.') || e.path() == search_path
            });

        let mut results: Vec<String> = Vec::new();
        let mut count = 0;

        for entry in walk {
            let entry = entry.map_err(|e| ToolError::Other(format!("Walk error: {e}")))?;

            if cancel.is_cancelled() {
                return Err(ToolError::Cancelled);
            }

            if !entry.file_type().is_file() {
                continue;
            }

            let rel_path = entry.path().strip_prefix(&search_path).unwrap_or(entry.path());
            let rel_str = rel_path.to_string_lossy();

            if re.is_match(rel_str.as_ref()) {
                results.push(rel_str.to_string());
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

        let mut output = format!("Found {} file(s) matching pattern: {pattern_str}\n\n", count);
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

/// Convert a glob pattern to a regex pattern.
/// Supports `*` (within one path component), `**` (cross-component), and `?` (single char).
fn glob_to_regex(glob: &str) -> Regex {
    let mut pattern = String::with_capacity(glob.len() + 8);
    pattern.push('^');

    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                // `**` - match across path separators
                pattern.push_str(".*");
                i += 2;
                // Skip any following `/`
                while i < chars.len() && chars[i] == '/' {
                    i += 1;
                }
            }
            '*' => {
                pattern.push_str("[^/]*");
                i += 1;
            }
            '?' => {
                pattern.push('.');
                i += 1;
            }
            '.' => {
                pattern.push_str("\\.");
                i += 1;
            }
            '/' => {
                pattern.push('/');
                i += 1;
            }
            '\\' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' => {
                pattern.push('\\');
                pattern.push(chars[i]);
                i += 1;
            }
            c => {
                pattern.push(c);
                i += 1;
            }
        }
    }
    pattern.push('$');

    Regex::new(&pattern).expect("Invalid glob regex conversion")
}
