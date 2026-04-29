use super::{
    atomic_write::write_atomic,
    diff,
    file_kind::{load_file, LoadedFile},
    hash::{compute_line_hash, file_hash, format_hashline_region, visible_lines},
    parse::{parse_line_ref, parse_lines_value, Anchor},
};
use crate::protocol::types::{ContentBlock, ToolContext, ToolDefinition, ToolResult};
use crate::tools::{registry::Tool, ToolError};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

pub struct HashlineEditTool;

#[derive(Clone)]
enum Op {
    Replace {
        pos: Anchor,
        end: Option<Anchor>,
        lines: Vec<String>,
    },
    Append {
        pos: Option<Anchor>,
        lines: Vec<String>,
    },
    Prepend {
        pos: Option<Anchor>,
        lines: Vec<String>,
    },
    ReplaceText {
        old: String,
        new: String,
    },
}
#[derive(Clone)]
struct Span {
    start: usize,
    end: usize,
    repl: String,
    line_start: usize,
    line_end: usize,
    kind: &'static str,
}

#[async_trait]
impl Tool for HashlineEditTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition { name: "hashline_edit".into(), description: "Edit an existing UTF-8 text file using LINE#HASH anchors copied from read output. Supports replace, append, prepend, and replace_text. Anchors must match current file content; replacement lines must be literal file content, not LINE#HASH or diff-prefixed lines. Applies validated edits atomically and returns diff preview plus updated anchors.".into(), input_schema: serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"intent":{"type":"string","description":"Why this edit is being made"},"scope":{"type":"string","enum":["localized","test","broad_refactor"],"default":"localized"},"edits":{"type":"array","minItems":1,"items":{"type":"object","properties":{"op":{"type":"string","enum":["replace","append","prepend","replace_text"]},"pos":{"type":"string"},"end":{"type":"string"},"lines":{"oneOf":[{"type":"array","items":{"type":"string"}},{"type":"string"},{"type":"null"}]},"oldText":{"type":"string"},"newText":{"type":"string"}},"required":["op"]}}},"required":["path","edits"]}) }
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
        let scope = arguments
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("localized");
        let edits_v = arguments
            .get("edits")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                ToolError::InvalidArguments("Missing or invalid 'edits' array".into())
            })?;
        if edits_v.is_empty() {
            return Err(ToolError::InvalidArguments(
                "'edits' array must not be empty".into(),
            ));
        }
        let ops = parse_ops(edits_v)?;
        let abs = ctx.cwd.join(path_str);
        let canonical = std::fs::canonicalize(&abs).map_err(|e| {
            ToolError::Other(format!("Cannot resolve path '{}': {e}", abs.display()))
        })?;
        let lock = super::queue::file_lock(&canonical);
        let _guard = lock.lock().await;
        let loaded = load_file(&canonical).await?;
        let content = match loaded { LoadedFile::Text(t) => t, LoadedFile::Directory => return Err(ToolError::EditFailed(format!("[E_DIRECTORY] Path is a directory: {}", path_str))), LoadedFile::Image(m) => return Err(ToolError::EditFailed(format!("[E_BINARY_FILE] Path is an image file ({m}); hashline_edit only supports UTF-8 text."))), LoadedFile::Binary(d) => return Err(ToolError::EditFailed(format!("[E_BINARY_FILE] Path is a binary file: {} ({d})", path_str))) };
        validate_generated_guard(&canonical, &content, scope)?;
        let old_hash = file_hash(&content);
        let spans = resolve_spans(&content, &ops)?;
        if spans.is_empty() {
            return Ok(ToolResult {
                content: vec![ContentBlock::Text {
                    text: format!("No changes needed for {}.", canonical.display()),
                }],
                is_error: false,
                metadata: Some(
                    serde_json::json!({"path": canonical.to_string_lossy(), "status":"noop", "oldFileHash": old_hash, "newFileHash": old_hash}),
                ),
            });
        }
        guard_large_edit(&content, &spans, scope)?;
        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let mut result = content.clone();
        let mut ordered = spans.clone();
        ordered.sort_by(|a, b| b.start.cmp(&a.start).then(b.end.cmp(&a.end)));
        for s in &ordered {
            result.replace_range(s.start..s.end, &s.repl);
        }
        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        write_atomic(&canonical, &result).await?;
        let changed = diff::changed_line_range(&content, &result);
        let new_hash = file_hash(&result);
        let diff_preview = diff::compact_diff(&content, &result, 80);
        let new_lines = visible_lines(&result);
        let (anchor_start, anchor_end, anchors) = if let Some((s, e)) = changed {
            let st = s.saturating_sub(2).max(1);
            let en = (e + 2).min(new_lines.len());
            let text = if st <= en && !new_lines.is_empty() {
                format_hashline_region(&new_lines[st - 1..en], st)
            } else {
                String::new()
            };
            (Some(st), Some(en), text)
        } else {
            (None, None, String::new())
        };
        let mut text = format!(
            "Applied {} edit(s) to {}.\n\nDiff preview:\n{}",
            spans.len(),
            canonical.display(),
            diff_preview
        );
        if !anchors.is_empty() {
            text.push_str(&format!(
                "\n\n--- Updated anchors {}-{} ---\n{}",
                anchor_start.unwrap(),
                anchor_end.unwrap(),
                anchors
            ));
        }
        Ok(ToolResult {
            content: vec![ContentBlock::Text { text }],
            is_error: false,
            metadata: Some(
                serde_json::json!({"path": canonical.to_string_lossy(), "status":"applied", "scope": scope, "oldFileHash": old_hash, "newFileHash": new_hash, "changedSpan": changed.map(|(s,e)| serde_json::json!({"start":s,"end":e})), "linesAdded": result.lines().count() as isize - content.lines().count() as isize, "diffPreview": diff_preview, "updatedAnchorsStart": anchor_start, "updatedAnchorsEnd": anchor_end }),
            ),
        })
    }
}

fn parse_ops(edits: &[serde_json::Value]) -> Result<Vec<Op>, ToolError> {
    let mut ops = Vec::new();
    for (i, e) in edits.iter().enumerate() {
        let op = e
            .get("op")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments(format!("edits[{i}]: missing op")))?;
        match op {
            "replace" => {
                let pos =
                    parse_line_ref(e.get("pos").and_then(|v| v.as_str()).ok_or_else(|| {
                        ToolError::InvalidArguments(format!(
                            "[E_BAD_OP] edits[{i}]: replace requires pos"
                        ))
                    })?)
                    .map_err(ToolError::InvalidArguments)?;
                let end = e
                    .get("end")
                    .and_then(|v| v.as_str())
                    .map(parse_line_ref)
                    .transpose()
                    .map_err(ToolError::InvalidArguments)?;
                let lines =
                    parse_lines_value(e.get("lines")).map_err(ToolError::InvalidArguments)?;
                ops.push(Op::Replace { pos, end, lines });
            }
            "append" => {
                let pos = e
                    .get("pos")
                    .and_then(|v| v.as_str())
                    .map(parse_line_ref)
                    .transpose()
                    .map_err(ToolError::InvalidArguments)?;
                if e.get("end").is_some() {
                    return Err(ToolError::InvalidArguments(
                        "[E_BAD_OP] append does not support end".into(),
                    ));
                }
                let lines =
                    parse_lines_value(e.get("lines")).map_err(ToolError::InvalidArguments)?;
                if lines.is_empty() {
                    return Err(ToolError::InvalidArguments(
                        "[E_BAD_OP] Append with empty lines payload.".into(),
                    ));
                }
                ops.push(Op::Append { pos, lines });
            }
            "prepend" => {
                let pos = e
                    .get("pos")
                    .and_then(|v| v.as_str())
                    .map(parse_line_ref)
                    .transpose()
                    .map_err(ToolError::InvalidArguments)?;
                if e.get("end").is_some() {
                    return Err(ToolError::InvalidArguments(
                        "[E_BAD_OP] prepend does not support end".into(),
                    ));
                }
                let lines =
                    parse_lines_value(e.get("lines")).map_err(ToolError::InvalidArguments)?;
                if lines.is_empty() {
                    return Err(ToolError::InvalidArguments(
                        "[E_BAD_OP] Prepend with empty lines payload.".into(),
                    ));
                }
                ops.push(Op::Prepend { pos, lines });
            }
            "replace_text" => {
                let old = e
                    .get("oldText")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidArguments(
                            "[E_BAD_OP] replace_text requires oldText".into(),
                        )
                    })?
                    .replace("\r\n", "\n")
                    .replace('\r', "\n");
                let new = e
                    .get("newText")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidArguments(
                            "[E_BAD_OP] replace_text requires newText".into(),
                        )
                    })?
                    .replace("\r\n", "\n")
                    .replace('\r', "\n");
                ops.push(Op::ReplaceText { old, new });
            }
            _ => {
                return Err(ToolError::InvalidArguments(format!(
                    "[E_BAD_OP] Unknown edit op {op:?}."
                )))
            }
        }
    }
    Ok(ops)
}

fn validate_anchor(
    a: &Anchor,
    lines: &[String],
    mismatches: &mut Vec<(usize, String, String)>,
) -> Result<bool, ToolError> {
    if a.line < 1 || a.line > lines.len() {
        return Err(ToolError::EditFailed(format!(
            "[E_RANGE_OOB] Line {} does not exist (file has {} lines)",
            a.line,
            lines.len()
        )));
    }
    let actual = compute_line_hash(a.line, &lines[a.line - 1]);
    if actual == a.hash {
        Ok(true)
    } else {
        mismatches.push((a.line, a.hash.clone(), actual));
        Ok(false)
    }
}
fn stale_error(m: &[(usize, String, String)], lines: &[String]) -> String {
    let mut nums = std::collections::BTreeSet::new();
    for (l, _, _) in m {
        for i in l.saturating_sub(2).max(1)..=(*l + 2).min(lines.len()) {
            nums.insert(i);
        }
    }
    let mut out = format!("[E_STALE_ANCHOR] {} stale anchor(s). Retry with the >>> LINE#HASH lines below; keep both endpoints for range replaces.\n", m.len());
    for n in nums {
        let prefix = if m.iter().any(|(l, _, _)| *l == n) {
            ">>>"
        } else {
            "   "
        };
        out.push_str(&format!(
            "\n{} {}",
            prefix,
            super::hash::render_hashline(n, &lines[n - 1])
        ));
    }
    out
}

fn build_index(content: &str) -> (Vec<String>, Vec<usize>) {
    let lines = visible_lines(content);
    let mut starts = Vec::new();
    let mut off = 0;
    for (i, l) in lines.iter().enumerate() {
        starts.push(off);
        off += l.len();
        if i < lines.len() - 1 || content.ends_with('\n') {
            off += 1;
        }
    }
    (lines, starts)
}
fn line_end(lines: &[String], starts: &[usize], line: usize, content: &str) -> usize {
    let idx = line - 1;
    let mut end = starts[idx] + lines[idx].len();
    if line < lines.len() && end < content.len() {
        end += 1;
    }
    end
}
fn resolve_spans(content: &str, ops: &[Op]) -> Result<Vec<Span>, ToolError> {
    let (lines, starts) = build_index(content);
    let mut mismatches = Vec::new();
    for op in ops {
        match op {
            Op::Replace { pos, end, .. } => {
                if let Some(e) = end {
                    if pos.line > e.line {
                        return Err(ToolError::InvalidArguments(format!(
                            "[E_BAD_OP] Range start line {} must be <= end line {}",
                            pos.line, e.line
                        )));
                    }
                    validate_anchor(pos, &lines, &mut mismatches)?;
                    validate_anchor(e, &lines, &mut mismatches)?;
                } else {
                    validate_anchor(pos, &lines, &mut mismatches)?;
                }
            }
            Op::Append { pos: Some(a), .. } | Op::Prepend { pos: Some(a), .. } => {
                validate_anchor(a, &lines, &mut mismatches)?;
            }
            _ => {}
        }
    }
    if !mismatches.is_empty() {
        return Err(ToolError::EditFailed(stale_error(&mismatches, &lines)));
    }
    let mut spans = Vec::new();
    for op in ops {
        match op {
            Op::Replace {
                pos,
                end,
                lines: repl,
            } => {
                let end_line = end.as_ref().map(|a| a.line).unwrap_or(pos.line);
                let start = starts[pos.line - 1];
                let replacement = repl.join("\n");
                let endb = if replacement.is_empty() {
                    line_end(&lines, &starts, end_line, content)
                } else {
                    starts[end_line - 1] + lines[end_line - 1].len()
                };
                spans.push(Span {
                    start,
                    end: endb,
                    repl: replacement,
                    line_start: pos.line,
                    line_end: end_line,
                    kind: "replace",
                });
            }
            Op::Append { pos, lines: repl } => {
                let txt = repl.join("\n");
                let (start, r) = if content.is_empty() {
                    (0, txt)
                } else if let Some(a) = pos {
                    let p = starts[a.line - 1] + lines[a.line - 1].len();
                    (p, format!("\n{}", txt))
                } else if content.ends_with('\n') {
                    (content.len(), format!("{}\n", txt))
                } else {
                    (content.len(), format!("\n{}", txt))
                };
                spans.push(Span {
                    start,
                    end: start,
                    repl: r,
                    line_start: pos.as_ref().map(|a| a.line + 1).unwrap_or(lines.len() + 1),
                    line_end: pos
                        .as_ref()
                        .map(|a| a.line + repl.len())
                        .unwrap_or(lines.len() + repl.len()),
                    kind: "insert",
                });
            }
            Op::Prepend { pos, lines: repl } => {
                let txt = repl.join("\n");
                let start = pos.as_ref().map(|a| starts[a.line - 1]).unwrap_or(0);
                let r = if content.is_empty() {
                    txt
                } else {
                    format!("{}\n", txt)
                };
                spans.push(Span {
                    start,
                    end: start,
                    repl: r,
                    line_start: pos.as_ref().map(|a| a.line).unwrap_or(1),
                    line_end: pos
                        .as_ref()
                        .map(|a| a.line + repl.len() - 1)
                        .unwrap_or(repl.len()),
                    kind: "insert",
                });
            }
            Op::ReplaceText { old, new } => {
                if old.is_empty() {
                    return Err(ToolError::InvalidArguments(
                        "[E_BAD_OP] replace_text requires non-empty oldText.".into(),
                    ));
                }
                let matches: Vec<_> = content
                    .match_indices(old.as_str())
                    .map(|(i, _)| i)
                    .collect();
                if matches.is_empty() {
                    return Err(ToolError::EditFailed("[E_NO_MATCH] replace_text found no exact unique match in the current file.".into()));
                }
                if matches.len() > 1 {
                    return Err(ToolError::EditFailed("[E_MULTI_MATCH] replace_text found multiple exact matches in the current file. Re-read and use hashline edits.".into()));
                }
                let st = matches[0];
                spans.push(Span {
                    start: st,
                    end: st + old.len(),
                    repl: new.clone(),
                    line_start: 1,
                    line_end: 1,
                    kind: "replace_text",
                });
            }
        }
    }
    spans.sort_by_key(|s| (s.start, s.end));
    for i in 1..spans.len() {
        if spans[i].start < spans[i - 1].end
            || (spans[i].start == spans[i - 1].start && spans[i].end == spans[i - 1].end)
        {
            return Err(ToolError::EditFailed("[E_OVERLAP] Edits overlap or target the same insertion boundary; merge them into one edit.".into()));
        }
    }
    Ok(spans)
}
fn validate_generated_guard(
    path: &std::path::Path,
    content: &str,
    scope: &str,
) -> Result<(), ToolError> {
    let p = path.to_string_lossy();
    let protected = [
        "/.git/",
        "/target/",
        "/build/",
        "/dist/",
        "/out/",
        "/bazel-out/",
        "/generated/",
        "/gen/",
    ];
    if scope != "broad_refactor" && protected.iter().any(|x| p.contains(x)) {
        return Err(ToolError::EditFailed(format!("[E_PROTECTED_PATH] Refusing to edit protected/generated path {} without broad_refactor scope/user approval.", path.display())));
    }
    let head = content.lines().take(20).collect::<Vec<_>>().join("\n");
    if scope != "broad_refactor"
        && [
            "DO NOT EDIT",
            "Generated by",
            "@generated",
            "Code generated",
        ]
        .iter()
        .any(|m| head.contains(m))
    {
        return Err(ToolError::EditFailed("[E_GENERATED_FILE] File appears generated. Ask user before editing or use broad_refactor scope.".into()));
    }
    Ok(())
}
fn guard_large_edit(content: &str, spans: &[Span], scope: &str) -> Result<(), ToolError> {
    if scope == "broad_refactor" {
        return Ok(());
    }
    let total = visible_lines(content).len().max(1);
    let removed: usize = spans
        .iter()
        .filter(|s| s.kind != "insert")
        .map(|s| s.line_end.saturating_sub(s.line_start) + 1)
        .sum();
    let added: usize = spans
        .iter()
        .map(|s| {
            if s.repl.is_empty() {
                0
            } else {
                s.repl.split('\n').count()
            }
        })
        .sum();
    let localized_removed_limit = (total * 30 / 100).max(10);
    if removed > localized_removed_limit || added > 160 || removed + added > 250 {
        return Err(ToolError::EditFailed("[E_EDIT_TOO_LARGE] Edit is too large for localized scope. Make a smaller edit or ask user for broad_refactor approval.".into()));
    }
    Ok(())
}
