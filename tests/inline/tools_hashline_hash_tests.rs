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
