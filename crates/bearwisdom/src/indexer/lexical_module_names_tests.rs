use super::*;

#[test]
fn quoted_names_decode_once_without_retaining_escape_spellings() {
    for (raw, expected) in [
        (r#"'provider'"#, "provider"),
        (r#""pro\u0076ider""#, "provider"),
        (r#"'pro\x76ider'"#, "provider"),
        (r#"'\u{1f600}'"#, "😀"),
        (r#"'\uD83D\uDE00'"#, "😀"),
        (r#"'a\'b'"#, "a'b"),
        (r#"'a\\b'"#, "a\\b"),
        ("'a\\\r\nb'", "ab"),
        (r#"''"#, ""),
    ] {
        assert_eq!(decode(raw).as_deref(), Some(expected), "{raw}");
    }
}

#[test]
fn malformed_or_unrepresentable_literals_are_not_raw_name_fallbacks() {
    for raw in [
        "provider",
        "'unterminated",
        "'a\nb'",
        r#"'\xq0'"#,
        r#"'\u{}'"#,
        r#"'\u{110000}'"#,
        r#"'\uD800'"#,
        r#"'\07'"#,
        r#"'\8'"#,
        "'a'b'",
    ] {
        assert_eq!(decode(raw), None, "{raw}");
    }
}
