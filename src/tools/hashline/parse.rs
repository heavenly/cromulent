use super::hash::alphabet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub line: usize,
    pub hash: String,
    pub text_hint: Option<String>,
}

pub fn parse_line_ref(input: &str) -> Result<Anchor, String> {
    let core = input
        .trim_start_matches(|c: char| c.is_whitespace() || c == '>' || c == '+' || c == '-')
        .trim_end();
    let Some(hash_pos) = core.find('#') else {
        if core
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace())
            && !core.trim().is_empty()
        {
            return Err(format!("[E_BAD_REF] Invalid line reference {:?}: missing hash, use \"LINE#HASH\" from read output (e.g. \"5#MQ\").", input));
        }
        return Err(format!(
            "[E_BAD_REF] Invalid line reference {:?}. Expected \"LINE#HASH\" (e.g. \"5#MQ\").",
            input
        ));
    };
    let line_str = core[..hash_pos].trim();
    let rest = core[hash_pos + 1..].trim_start();
    let (hash, hint) = match rest.find(':') {
        Some(i) => (&rest[..i].trim(), Some(rest[i + 1..].to_string())),
        None => (&rest.trim(), None),
    };
    let line: usize = line_str.parse().map_err(|_| {
        format!(
            "[E_BAD_REF] Invalid line reference {:?}. Expected \"LINE#HASH\".",
            input
        )
    })?;
    if line < 1 {
        return Err(format!(
            "[E_BAD_REF] Line number must be >= 1, got {line} in {:?}.",
            input
        ));
    }
    if hash.len() != 2 {
        return Err(format!(
            "[E_BAD_REF] Invalid line reference {:?}: hash must be exactly 2 characters from {}.",
            input,
            alphabet()
        ));
    }
    if !hash.chars().all(|c| alphabet().contains(c)) {
        return Err(format!("[E_BAD_REF] Invalid line reference {:?}: hash uses invalid characters, hashes use alphabet {} only.", input, alphabet()));
    }
    Ok(Anchor {
        line,
        hash: hash.to_string(),
        text_hint: hint,
    })
}

fn is_hash_char(c: char) -> bool {
    alphabet().contains(c)
}

pub fn reject_display_prefixes(lines: &[String]) -> Result<(), String> {
    for line in lines {
        let t = line.trim_start();
        let t = t
            .strip_prefix(">>>")
            .or_else(|| t.strip_prefix(">>"))
            .unwrap_or(t)
            .trim_start();
        let t_plus = t.strip_prefix('+').map(|s| s.trim_start()).unwrap_or(t);
        if looks_hashline_prefix(t) || looks_hashline_prefix(t_plus) || looks_diff_minus(line) {
            return Err(format!("[E_INVALID_PATCH] \"lines\" must contain literal file content, not rendered \"LINE#HASH:\" or diff \"+/-\" prefixes. Offending line: {:?}", line));
        }
    }
    Ok(())
}

fn looks_hashline_prefix(s: &str) -> bool {
    let mut chars = s.chars().peekable();
    let mut saw_digit = false;
    while matches!(chars.peek(), Some(c) if c.is_ascii_digit()) {
        saw_digit = true;
        chars.next();
    }
    while matches!(chars.peek(), Some(c) if c.is_whitespace()) {
        chars.next();
    }
    if chars.peek() == Some(&'#') {
        chars.next();
    } else {
        return false;
    }
    while matches!(chars.peek(), Some(c) if c.is_whitespace()) {
        chars.next();
    }
    let h1 = chars.next();
    let h2 = chars.next();
    saw_digit
        && h1.map(is_hash_char).unwrap_or(false)
        && h2.map(is_hash_char).unwrap_or(false)
        && chars.peek() == Some(&':')
}
fn looks_diff_minus(s: &str) -> bool {
    let t = s.trim_start();
    if !t.starts_with('-') {
        return false;
    }
    let r = &t[1..];
    let digits = r.chars().take_while(|c| c.is_ascii_digit()).count();
    digits > 0 && r[digits..].starts_with("    ")
}

pub fn parse_lines_value(value: Option<&serde_json::Value>) -> Result<Vec<String>, String> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(vec![]),
        Some(serde_json::Value::String(s)) => {
            let s = s.replace("\r\n", "\n").replace('\r', "\n");
            let s = s.strip_suffix('\n').unwrap_or(&s).to_string();
            let lines = s.split('\n').map(|x| x.to_string()).collect::<Vec<_>>();
            reject_display_prefixes(&lines)?;
            Ok(lines)
        }
        Some(serde_json::Value::Array(a)) => {
            let mut out = Vec::with_capacity(a.len());
            for v in a {
                out.push(
                    v.as_str()
                        .ok_or_else(|| {
                            "[E_INVALID_PATCH] lines array must contain only strings".to_string()
                        })?
                        .to_string(),
                );
            }
            reject_display_prefixes(&out)?;
            Ok(out)
        }
        _ => Err("[E_INVALID_PATCH] lines must be a string, string array, or null".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_rendered() {
        assert_eq!(parse_line_ref("12#MQ:abc").unwrap().line, 12);
    }
    #[test]
    fn rejects_prefix() {
        assert!(reject_display_prefixes(&["12#MQ:abc".into()]).is_err());
        assert!(reject_display_prefixes(&["+ 12#MQ:abc".into()]).is_err());
    }
}
