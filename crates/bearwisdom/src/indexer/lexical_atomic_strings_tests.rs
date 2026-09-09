use super::*;
fn value(text: &str) -> LitValue {
    decode(text, &crate::languages::typescript::flow::ATOMIC_TYPES).unwrap()
}

#[test]
fn escaped_and_utf8_string_literals_have_canonical_value_identity() {
    assert_eq!(value(r#"'a\u0062\x63'"#), value(r#""abc""#));
    assert_eq!(value(r#"'\u{1f600}'"#), value("'😀'"));
    assert_eq!(value(r#"'\ud83d\ude00'"#), value("'😀'"));
    assert_eq!(value("'a\\\r\nb'"), value("'ab'"));
    assert_eq!(value(r#"'\a'"#), value("'a'"));
    assert_eq!(value(r#"'\u{000000000061}'"#), value("'a'"));
}

#[test]
fn lone_surrogates_do_not_collapse_to_replacement_characters() {
    assert_eq!(value(r#"'\ud800'"#), LitValue::Utf16(vec![0xd800]));
    assert_eq!(value(r#"'\u{d800}'"#), LitValue::Utf16(vec![0xd800]));
    assert_ne!(value(r#"'\ud800'"#), value(r#"'\ud801'"#));
    assert_ne!(value(r#"'\ud800'"#), value("'�'"));
    assert!(decode(
        r#"'\u{110000}'"#,
        &crate::languages::typescript::flow::ATOMIC_TYPES
    )
    .is_none());
}
