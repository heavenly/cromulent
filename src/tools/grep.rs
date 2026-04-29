use async_trait::async_trait;
use regex::Regex;
use tokio_util::sync::CancellationToken;

use crate::protocol::types::{ContentBlock, ToolContext, ToolDefinition, ToolResult};
use crate::tools::registry::Tool;
use crate::tools::ToolError;

/// Searches file contents for a pattern (regex or literal), with optional glob filtering.
pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "grep".to_string(),
            description: "Search file contents for a pattern. Supports regex or literal search, with optional glob-based file filtering and context lines.".to_string(),
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
            Regex::new(&format!("(?i){}", re_str))?
        } else {
            Regex::new(&re_str)?
        };

        // Walk the search path, collect matching files
        let walk = walkdir::WalkDir::new(&search_path)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                // Skip hidden dirs/files unless search path is explicitly a file
                let fname = e.file_name().to_string_lossy();
                !fname.starts_with('.') || e.path() == search_path
            });

        let mut results: Vec<String> = Vec::new();
        let mut match_count = 0;

        for entry in walk {
            let entry = entry.map_err(|e| ToolError::Other(format!("Walk error: {e}")))?;

            if cancel.is_cancelled() {
                return Err(ToolError::Cancelled);
            }

            if !entry.file_type().is_file() {
                continue;
            }

            // Apply glob filter
            if let Some(glob) = glob_filter {
                let rel_path = entry
                    .path()
                    .strip_prefix(&search_path)
                    .unwrap_or(entry.path());
                if !simple_glob_match(glob, rel_path.to_string_lossy().as_ref()) {
                    continue;
                }
            }

            // Read the file
            let content = match tokio::fs::read_to_string(entry.path()).await {
                Ok(c) => c,
                Err(_) => continue, // Skip binary/unreadable files
            };

            if cancel.is_cancelled() {
                return Err(ToolError::Cancelled);
            }

            let lines: Vec<&str> = content.lines().collect();
            let mut file_matches: Vec<(usize, String)> = Vec::new();

            for (i, line) in lines.iter().enumerate() {
                if re.is_match(line) {
                    file_matches.push((i, line.to_string()));
                }
            }

            if file_matches.is_empty() {
                continue;
            }

            // Build output with context
            for (line_idx, _matched_line) in &file_matches {
                if match_count >= max_matches {
                    break;
                }

                let ctx_start = if *line_idx >= context_lines {
                    line_idx - context_lines
                } else {
                    0
                };
                let ctx_end = (line_idx + 1 + context_lines).min(lines.len());

                let mut block = String::new();
                block.push_str(&format!(
                    "{} (lines {}-{}):\n",
                    entry.path().display(),
                    ctx_start + 1,
                    ctx_end
                ));
                for ci in ctx_start..ctx_end {
                    let prefix = if ci == *line_idx { ">" } else { " " };
                    block.push_str(&format!("{}{}: {}\n", prefix, ci + 1, lines[ci]));
                }
                block.push('\n');

                results.push(block);
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

/// Simple glob-to-path matching.
/// Supports `*` (within one path component), `**` (cross-component), and `?` (single char).
fn simple_glob_match(glob: &str, path: &str) -> bool {
    // Convert glob pattern to regex
    let mut re_str = String::with_capacity(glob.len() + 4);
    re_str.push('^');
    for ch in glob.chars() {
        match ch {
            '*' => re_str.push_str("[^/]*"),
            '?' => re_str.push('.'),
            '.' => re_str.push_str("\\."),
            '/' => re_str.push('/'),
            // Escape other regex meta-characters
            '\\' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' => {
                re_str.push('\\');
                re_str.push(ch);
            }
            other => re_str.push(other),
        }
    }
    re_str.push('$');

    // Handle double-star: replace `[^/]*[^/]*`  or just handle it at a higher level
    // Actually our current conversion makes `*` -> `[^/]*`, so `**` -> `[^/]*[^/]*` which matches
    // across path separators correctly because `[^/]*` matches anything except `/`.
    // Wait, that's wrong. `**` should match across `/`, but our `*` already only matches within one component.
    // Let me fix: replace `**` with `.*` before processing
    let fixed_glob = glob.replace("**", "$$DOUBLESTAR$$");
    let mut re_str2 = String::with_capacity(fixed_glob.len() + 4);
    re_str2.push('^');
    for ch in fixed_glob.chars() {
        match ch {
            '*' => re_str2.push_str("[^/]*"),
            '?' => re_str2.push('.'),
            '.' => re_str2.push_str("\\."),
            '/' => re_str2.push('/'),
            '$' => {
                // Check if it's the start of our placeholder
                // Simple approach: just handle the placeholder char by char
                // $$DOUBLESTAR$$ -> we need to handle the '$' specially
                re_str2.push_str(".*");
                // Skip the rest of the placeholder
                // Hmm, this is getting messy. Let me use a simpler approach.
                // Actually, this won't work well with char-by-char iteration.
                // Let me use a different approach below.
            }
            '\\' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '|' => {
                re_str2.push('\\');
                re_str2.push(ch);
            }
            other => re_str2.push(other),
        }
    }
    re_str2.push('$');

    // Hmm, the above approach is flawed with the placeholder. Let me use string replacement instead.
    // Re-do the whole thing properly.
    let re = build_glob_regex(glob);
    re.is_match(path)
}

fn build_glob_regex(glob: &str) -> Regex {
    // Handle `**` first: replace with a sentinel
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
