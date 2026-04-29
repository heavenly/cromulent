use super::hash::{render_hashline, visible_lines};

pub fn changed_line_range(old: &str, new: &str) -> Option<(usize, usize)> {
    if old == new {
        return None;
    }
    let old_lines = visible_lines(old);
    let new_lines = visible_lines(new);
    let min = old_lines.len().min(new_lines.len());
    let mut first = 0usize;
    while first < min && old_lines[first] == new_lines[first] {
        first += 1;
    }
    let mut old_last = old_lines.len();
    let mut new_last = new_lines.len();
    while old_last > first && new_last > first && old_lines[old_last - 1] == new_lines[new_last - 1]
    {
        old_last -= 1;
        new_last -= 1;
    }
    let start = first + 1;
    let end = new_last.max(start);
    Some((start, end))
}

pub fn compact_diff(old: &str, new: &str, max_lines: usize) -> String {
    let old_lines = visible_lines(old);
    let new_lines = visible_lines(new);
    let Some((start, end_new)) = changed_line_range(old, new) else {
        return "(no changes)".into();
    };
    let end_old = {
        let mut old_last = old_lines.len();
        let mut new_last = new_lines.len();
        let first = start - 1;
        while old_last > first
            && new_last > first
            && old_lines[old_last - 1] == new_lines[new_last - 1]
        {
            old_last -= 1;
            new_last -= 1;
        }
        old_last
    };
    let mut out = Vec::new();
    for i in start..=end_old {
        if let Some(l) = old_lines.get(i - 1) {
            out.push(format!("- {}", render_hashline(i, l)));
            if out.len() >= max_lines {
                out.push("...".into());
                return out.join("\n");
            }
        }
    }
    for i in start..=end_new {
        if let Some(l) = new_lines.get(i - 1) {
            out.push(format!("+ {}", render_hashline(i, l)));
            if out.len() >= max_lines {
                out.push("...".into());
                break;
            }
        }
    }
    out.join("\n")
}
