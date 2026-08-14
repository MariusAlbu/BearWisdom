use super::strip_scheme_prefix;

#[test]
fn node_scheme_strips() {
    assert_eq!(strip_scheme_prefix("node:assert/strict"), Some("assert/strict"));
    assert_eq!(strip_scheme_prefix("node:fs"), Some("fs"));
    assert_eq!(strip_scheme_prefix("jsr:@std/path"), Some("@std/path"));
}

#[test]
fn qualified_paths_are_not_schemes() {
    assert_eq!(strip_scheme_prefix("crate::db"), None);
    assert_eq!(strip_scheme_prefix("std::collections::HashMap"), None);
}

#[test]
fn non_scheme_shapes_pass_through() {
    assert_eq!(strip_scheme_prefix("assert/strict"), None);
    assert_eq!(strip_scheme_prefix("./relative"), None);
    assert_eq!(strip_scheme_prefix(":leading"), None);
    assert_eq!(strip_scheme_prefix("node:"), None);
    assert_eq!(strip_scheme_prefix("1um:x"), None);
}
