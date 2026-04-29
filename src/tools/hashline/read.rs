use super::hash::{file_hash, format_hashline_region, visible_lines};

pub struct Preview {
    pub text: String,
    pub total_lines: usize,
    pub returned: usize,
    pub truncated: bool,
    pub next_offset: Option<usize>,
    pub start: usize,
    pub end: usize,
    pub file_hash: String,
}

pub fn format_read_preview(
    text: &str,
    offset: usize,
    limit: Option<usize>,
) -> Result<Preview, String> {
    let lines = visible_lines(text);
    let total = lines.len();
    let start = offset.max(1);
    if total == 0 {
        return Ok(Preview {
            text: if start == 1 {
                "File is empty. Use hashline_edit with prepend or append and omit pos to insert content.".into()
            } else {
                format!("Offset {start} is beyond end of file (0 lines total). The file is empty. Use hashline_edit with prepend or append and omit pos to insert content.")
            },
            total_lines: 0,
            returned: 0,
            truncated: false,
            next_offset: None,
            start,
            end: 0,
            file_hash: file_hash(text),
        });
    }
    if start > total {
        return Ok(Preview { text: format!("Offset {start} is beyond end of file ({total} lines total). Use offset=1 to read from the start, or offset={total} to read the last line."), total_lines: total, returned: 0, truncated: false, next_offset: None, start, end: total, file_hash: file_hash(text) });
    }
    let default_limit = 200usize;
    let lim = limit.unwrap_or(default_limit).max(1);
    let end = (start - 1 + lim).min(total);
    let selected = lines[start - 1..end].to_vec();
    let mut out = format_hashline_region(&selected, start);
    let truncated = end < total;
    let next = if truncated { Some(end + 1) } else { None };
    if let Some(n) = next {
        out.push_str(&format!(
            "\n\n[Showing lines {start}-{end} of {total}. Use offset={n} to continue.]"
        ));
    }
    Ok(Preview {
        text: out,
        total_lines: total,
        returned: selected.len(),
        truncated,
        next_offset: next,
        start,
        end,
        file_hash: file_hash(text),
    })
}
