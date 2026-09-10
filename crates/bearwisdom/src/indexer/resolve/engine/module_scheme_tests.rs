use super::{node_builtin_module_alias, strip_scheme_prefix};

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

#[test]
fn node_builtin_alias_accepts_builtin_and_subpath() {
    assert_eq!(node_builtin_module_alias("node:path"), Some("path"));
    assert_eq!(
        node_builtin_module_alias("node:fs/promises"),
        Some("fs/promises")
    );
    assert_eq!(
        node_builtin_module_alias("node:diagnostics_channel"),
        Some("diagnostics_channel")
    );
}

#[test]
fn node_builtin_alias_rejects_other_schemes_and_malformed_paths() {
    for spec in [
        "path",
        "sass:math",
        "jsr:@std/path",
        "node:",
        "node:/path",
        "node:path/",
        "node:fs//promises",
        "node:./path",
        "node:../path",
        "node:fs/../path",
        "node:fs?query",
        "node:fs\\path",
    ] {
        assert_eq!(node_builtin_module_alias(spec), None, "{spec}");
    }
}
