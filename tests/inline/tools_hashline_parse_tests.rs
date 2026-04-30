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
