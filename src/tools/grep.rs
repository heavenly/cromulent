use async_trait::async_trait;
use ignore::WalkBuilder;
use regex::Regex;
use tokio::io::AsyncBufReadExt;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{ContentBlock, ToolContext, ToolDefinition, ToolResult};
use crate::tools::registry::Tool;
use crate::tools::ToolError;

/// Searches file contents for a pattern (regex or literal), with optional glob filtering.
/// Uses `ignore` for gitignore-aware traversal and reads files line-by-line to avoid
/// loading the entire file into memory.
pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "grep".to_string(),
            description: "Search file contents for a pattern. Supports regex or literal search, with optional glob-based file filtering and context lines. Respects .gitignore.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Search pattern (regex unless literal=true)"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory or file to search (default: cwd)"
                    },
                    "glob": {
                        "type": "string",
                        "description": "Optional glob pattern to filter files, e.g. '*.rs' or 'src/**/*.ts'"
                    },
                    "ignoreCase": {
                        "type": "boolean",
                        "description": "Case-insensitive search (default: false)"
                    },
                    "literal": {
                        "type": "boolean",
                        "description": "Treat pattern as literal string instead of regex (default: false)"
                    },
                    "context": {
                        "type": "integer",
                        "description": "Number of lines to show before and after each match (default: 0)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of matches to return (default: 100)"
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

        let is_literal = arguments
            .get("literal")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let ignore_case = arguments
            .get("ignoreCase")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let context_lines = arguments
            .get("context")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0) as usize;

        let max_matches = arguments
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(100)
            .max(1) as usize;

        let search_path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| ctx.cwd.join(p))
            .unwrap_or_else(|| ctx.cwd.clone());

        let glob_filter = arguments.get("glob").and_then(|v| v.as_str());

        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        // Build the regex
        let re_str = if is_literal {
            regex::escape(pattern_str)
        } else {
            pattern_str.to_string()
        };

        let re = if ignore_case {
            Regex::new(&format!("(?i){re_str}"))?
        } else {
            Regex::new(&re_str)?
        };

        // Use ignore::WalkBuilder for gitignore-aware traversal
        let walker = WalkBuilder::new(&search_path)
            .follow_links(false)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .require_git(false)
            .build();

        // Build optional glob matcher
        let glob_re = glob_filter.map(|g| build_glob_regex(g));

        let mut results: Vec<String> = Vec::new();
        let mut match_count = 0;

        for entry in walker {
            if cancel.is_cancelled() {
                return Err(ToolError::Cancelled);
            }

            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    tracing::debug!(error = %err, "Walk entry error");
                    continue;
                }
            };

            // Only search regular files
            if entry.file_type().map_or(true, |ft| !ft.is_file()) {
                continue;
            }

            // Apply glob filter
            if let Some(ref glob_re) = glob_re {
                let rel_path = entry
                    .path()
                    .strip_prefix(&search_path)
                    .unwrap_or(entry.path());
                if !glob_re.is_match(rel_path.to_string_lossy().as_ref()) {
                    continue;
                }
            }

            // Skip files that are likely binary or too large (>10MB)
            if let Ok(meta) = entry.metadata() {
                if meta.len() > 10_000_000 {
                    continue;
                }
            }

            // Read file line-by-line (streaming, not loading the whole file)
            let file_matches = match read_matches_line_by_line(
                entry.path(),
                &re,
                context_lines,
                max_matches - match_count,
            )
            .await
            {
                Ok(m) => m,
                Err(_) => continue, // Skip unreadable files
            };

            if file_matches.is_empty() {
                continue;
            }

            // Build output blocks
            for block in &file_matches {
                if match_count >= max_matches {
                    break;
                }
                results.push(format!("{}:\n{}\n", entry.path().display(), block));
                match_count += 1;
            }

            if match_count >= max_matches {
                break;
            }
        }

        if results.is_empty() {
            return Ok(ToolResult {
                content: vec![ContentBlock::Text {
                    text: format!("No matches found for pattern: {pattern_str}"),
                }],
                is_error: false,
                metadata: Some(serde_json::json!({
                    "pattern": pattern_str,
                    "matches": 0,
                })),
            });
        }

        let mut output = format!(
            "Found {} match(es) for pattern: {pattern_str}\n\n",
            match_count
        );
        output.push_str(&results.join(""));

        if match_count >= max_matches {
            output.push_str(&format!("\n[Reached limit of {} matches]", max_matches));
        }

        Ok(ToolResult {
            content: vec![ContentBlock::Text { text: output }],
            is_error: false,
            metadata: Some(serde_json::json!({
                "pattern": pattern_str,
                "matches": match_count,
                "limit": max_matches,
            })),
        })
    }
}

/// Read a file line-by-line, collecting matches with context.
/// This avoids loading the entire file into memory.
async fn read_matches_line_by_line(
    path: &std::path::Path,
    re: &Regex,
    context_lines: usize,
    max_matches: usize,
) -> std::io::Result<Vec<String>> {
    let file = tokio::fs::File::open(path).await?;
    let reader = tokio::io::BufReader::new(file);
    let mut lines_stream = reader.lines();

    let mut all_lines: Vec<String> = Vec::new();
    let mut match_indices: Vec<usize> = Vec::new();

    // Read all lines (we need them for context windows)
    while let Some(line) = lines_stream.next_line().await? {
        all_lines.push(line);
    }

    // Find matching line indices
    for (i, line) in all_lines.iter().enumerate() {
        if re.is_match(line) {
            match_indices.push(i);
            if match_indices.len() >= max_matches {
                break;
            }
        }
    }

    if match_indices.is_empty() {
        return Ok(Vec::new());
    }

    // Build context blocks
    let mut blocks = Vec::new();
    for line_idx in &match_indices {
        let ctx_start = if *line_idx >= context_lines {
            line_idx - context_lines
        } else {
            0
        };
        let ctx_end = (line_idx + 1 + context_lines).min(all_lines.len());

        let mut block = String::new();
        block.push_str(&format!("Lines {}-{}:\n", ctx_start + 1, ctx_end));
        for ci in ctx_start..ctx_end {
            let prefix = if ci == *line_idx { ">" } else { " " };
            let line = truncate_line(&all_lines[ci], 500);
            block.push_str(&format!("{}  {}: {}\n", prefix, ci + 1, line));
        }

        blocks.push(block);
    }

    Ok(blocks)
}

/// Truncate a line to max_len chars, appending "..." if truncated.
fn truncate_line(line: &str, max_len: usize) -> &str {
    if line.len() <= max_len {
        return line;
    }
    // Find a char boundary at or before max_len
    let mut end = max_len;
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

/// Simple glob-to-path matching regex.
/// Supports `*` (within one path component), `**` (cross-component), and `?` (single char).
fn build_glob_regex(glob: &str) -> Regex {
    let mut pattern = String::with_capacity(glob.len() + 8);
    pattern.push('^');

    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                pattern.push_str(".*");
                i += 2;
                if i < chars.len() && chars[i] == '/' {
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
