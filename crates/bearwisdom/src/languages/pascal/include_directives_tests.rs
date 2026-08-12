use super::*;

// End-to-end scanning behavior (long/short form, subdirectories, the `I+`/`I-`
// switch) is already covered via `pascal::extract::extract` in
// `extract_tests.rs`; these tests exercise the moved helpers directly.

#[test]
fn parses_long_form_directive() {
    assert_eq!(
        parse_include_directive("include 'shared_defs.inc'"),
        Some("shared_defs".to_string())
    );
}

#[test]
fn parses_short_form_directive() {
    assert_eq!(
        parse_include_directive("i helpers.inc"),
        Some("helpers".to_string())
    );
}

#[test]
fn rejects_io_check_switch() {
    assert_eq!(parse_include_directive("I+"), None);
    assert_eq!(parse_include_directive("I-"), None);
}

#[test]
fn file_stem_of_strips_directory_and_extension() {
    assert_eq!(
        file_stem_of("inc/shared_defs.inc"),
        Some("shared_defs".to_string())
    );
}

#[test]
fn line_of_byte_counts_preceding_newlines() {
    let src = "line0\nline1\nline2";
    assert_eq!(line_of_byte(src, 0), 0);
    assert_eq!(line_of_byte(src, 6), 1);
    assert_eq!(line_of_byte(src, 12), 2);
}

#[test]
fn extract_include_directives_emits_imports_ref_with_stem_and_line() {
    let src = "unit Foo;\n{$include 'helpers.inc'}\n";
    let mut refs = Vec::new();
    extract_include_directives(src, &mut refs);

    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].target_name, "helpers");
    assert_eq!(refs[0].module.as_deref(), Some("helpers"));
    assert_eq!(refs[0].kind, EdgeKind::Imports);
    assert_eq!(refs[0].line, 1);
}
