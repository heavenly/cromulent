const ALPHABET: &[u8; 16] = b"ZPMQVRWSNKTXJBYH";
const PRIME32_1: u32 = 0x9E3779B1;
const PRIME32_2: u32 = 0x85EBCA77;
const PRIME32_3: u32 = 0xC2B2AE3D;
const PRIME32_4: u32 = 0x27D4EB2F;
const PRIME32_5: u32 = 0x165667B1;

pub fn alphabet() -> &'static str {
    "ZPMQVRWSNKTXJBYH"
}

fn round(acc: u32, input: u32) -> u32 {
    acc.wrapping_add(input.wrapping_mul(PRIME32_2))
        .rotate_left(13)
        .wrapping_mul(PRIME32_1)
}

fn xxh32(bytes: &[u8], seed: u32) -> u32 {
    let len = bytes.len() as u32;
    let mut i = 0usize;
    let mut h32;
    if bytes.len() >= 16 {
        let mut v1 = seed.wrapping_add(PRIME32_1).wrapping_add(PRIME32_2);
        let mut v2 = seed.wrapping_add(PRIME32_2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(PRIME32_1);
        while i <= bytes.len() - 16 {
            v1 = round(v1, u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()));
            i += 4;
            v2 = round(v2, u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()));
            i += 4;
            v3 = round(v3, u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()));
            i += 4;
            v4 = round(v4, u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()));
            i += 4;
        }
        h32 = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
    } else {
        h32 = seed.wrapping_add(PRIME32_5);
    }
    h32 = h32.wrapping_add(len);
    while i + 4 <= bytes.len() {
        h32 = h32.wrapping_add(
            u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()).wrapping_mul(PRIME32_3),
        );
        h32 = h32.rotate_left(17).wrapping_mul(PRIME32_4);
        i += 4;
    }
    while i < bytes.len() {
        h32 = h32.wrapping_add((bytes[i] as u32).wrapping_mul(PRIME32_5));
        h32 = h32.rotate_left(11).wrapping_mul(PRIME32_1);
        i += 1;
    }
    h32 ^= h32 >> 15;
    h32 = h32.wrapping_mul(PRIME32_2);
    h32 ^= h32 >> 13;
    h32 = h32.wrapping_mul(PRIME32_3);
    h32 ^= h32 >> 16;
    h32
}

pub fn compute_line_hash(line_number: usize, line: &str) -> String {
    let normalized = line.replace('\r', "");
    let trimmed = normalized.trim_end();
    let seed = if trimmed.chars().any(|c| c.is_alphanumeric()) {
        0
    } else {
        line_number as u32
    };
    let low = (xxh32(trimmed.as_bytes(), seed) & 0xff) as usize;
    let hi = low >> 4;
    let lo = low & 0x0f;
    format!("{}{}", ALPHABET[hi] as char, ALPHABET[lo] as char)
}

pub fn render_hashline(line_number: usize, line: &str) -> String {
    format!(
        "{}#{}:{}",
        line_number,
        compute_line_hash(line_number, line),
        line
    )
}

pub fn format_hashline_region(lines: &[String], start_line: usize) -> String {
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| render_hashline(start_line + i, line))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn visible_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}

pub fn file_hash(text: &str) -> String {
    format!("{:08x}", xxh32(text.as_bytes(), 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_normalizes_trailing_ws_and_cr() {
        assert_eq!(compute_line_hash(1, "abc  \r"), compute_line_hash(1, "abc"));
    }
    #[test]
    fn symbol_lines_seeded_by_line() {
        assert_ne!(compute_line_hash(1, "}"), compute_line_hash(2, "}"));
    }
    #[test]
    fn render_prefix() {
        assert!(render_hashline(3, "let x = 1;").starts_with("3#"));
    }
}
