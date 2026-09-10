use super::*;

#[test]
fn node_builtin_alias_accepts_direct_and_nested_builtin_modules() {
    assert_eq!(node_builtin_module_alias("node:path"), Some("path"));
    assert_eq!(
        node_builtin_module_alias("node:fs/promises"),
        Some("fs/promises")
    );
    assert_eq!(
        node_builtin_module_alias("node:assert/strict"),
        Some("assert/strict")
    );
}

#[test]
fn node_builtin_alias_rejects_malformed_or_non_node_specifiers() {
    for specifier in [
        "path",
        "node:",
        "node:../path",
        "node:path/../fs",
        "node:path//posix",
        "node:fs.promises",
        "node:node:path",
    ] {
        assert_eq!(
            node_builtin_module_alias(specifier),
            None,
            "must not grant node builtin authority to {specifier:?}"
        );
    }
}

#[test]
fn node_builtin_declaration_root_is_exact_and_separator_stable() {
    assert!(is_node_builtin_declaration_path(
        "ext:ts:@types/node/path.d.ts"
    ));
    assert!(is_node_builtin_declaration_path(
        "ext:ts:@types\\node\\fs.d.ts"
    ));
    assert!(!is_node_builtin_declaration_path(
        "ext:ts:@types/nodeish/path.d.ts"
    ));
    assert!(!is_node_builtin_declaration_path(
        "ext:ts:node/path.d.ts"
    ));
    assert!(!is_node_builtin_declaration_path(
        "src/@types/node/path.d.ts"
    ));
}

#[test]
fn module_match_describes_declaration_suffixes_and_node_authority() {
    use crate::type_checker::profile::language_profile::ModuleMatchAuthority;

    let builtin = module_path_match("node:fs/promises");
    assert_eq!(builtin.module_path, "fs/promises");
    assert_eq!(builtin.required_file_prefix, Some(NODE_TYPES_VIRTUAL_ROOT));
    assert_eq!(
        builtin.compound_extensions,
        &[".d.ts", ".d.mts", ".d.cts"]
    );
    assert_eq!(builtin.authority, ModuleMatchAuthority::Authoritative);

    let ordinary = module_path_match("react");
    assert_eq!(ordinary.module_path, "react");
    assert_eq!(ordinary.authority, ModuleMatchAuthority::Heuristic);

    let malformed = module_path_match("node:../path");
    assert_eq!(malformed.authority, ModuleMatchAuthority::Reject);
}
