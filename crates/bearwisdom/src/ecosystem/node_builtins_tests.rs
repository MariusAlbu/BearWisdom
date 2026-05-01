// =============================================================================
// node_builtins_tests.rs — sibling tests for ecosystem/node_builtins.rs
// =============================================================================

use super::*;
use crate::types::SymbolKind;

fn all_symbols() -> Vec<crate::types::ExtractedSymbol> {
    _test_synthesize_all()
        .into_iter()
        .flat_map(|pf| pf.symbols)
        .collect()
}

#[test]
fn fs_read_file_present() {
    let syms = all_symbols();
    assert!(
        syms.iter().any(|s| s.qualified_name == "fs.readFile"),
        "expected fs.readFile in synthesized symbols"
    );
}

#[test]
fn path_join_present() {
    let syms = all_symbols();
    assert!(
        syms.iter().any(|s| s.qualified_name == "path.join"),
        "expected path.join in synthesized symbols"
    );
}

#[test]
fn node_prefix_alias_works() {
    let syms = all_symbols();
    assert!(
        syms.iter().any(|s| s.qualified_name == "node:fs.readFile"),
        "expected node:fs.readFile alias in synthesized symbols"
    );
    assert!(
        syms.iter().any(|s| s.qualified_name == "node:path.join"),
        "expected node:path.join alias in synthesized symbols"
    );
}

#[test]
fn symbol_count_reasonable() {
    let syms = all_symbols();
    assert!(syms.len() >= 68, "expected >= 68 symbols, got {}", syms.len());
}

#[test]
fn class_kinds_correct() {
    let syms = all_symbols();
    let event_emitter = syms
        .iter()
        .find(|s| s.qualified_name == "events.EventEmitter")
        .expect("events.EventEmitter must exist");
    assert_eq!(event_emitter.kind, SymbolKind::Class);

    let url_class = syms
        .iter()
        .find(|s| s.qualified_name == "url.URL")
        .expect("url.URL must exist");
    assert_eq!(url_class.kind, SymbolKind::Class);
}

#[test]
fn all_modules_synthesized() {
    let parsed = _test_synthesize_all();
    assert_eq!(
        parsed.len(),
        34,
        "expected 34 ParsedFiles (17 modules x 2 prefixes), got {}",
        parsed.len()
    );
}

#[test]
fn virtual_paths_follow_convention() {
    let parsed = _test_synthesize_all();
    for pf in &parsed {
        assert!(
            pf.path.starts_with("ext:node-builtins:"),
            "virtual path must start with ext:node-builtins: — got {}",
            pf.path
        );
    }
}

// ---------------------------------------------------------------------------
// has_types_node gate
// ---------------------------------------------------------------------------

#[test]
fn has_types_node_false_when_absent() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    assert!(!_test_has_types_node(tmp.path()));
}

#[test]
fn has_types_node_true_for_direct_node_modules() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let dir = tmp.path().join("node_modules/@types/node");
    std::fs::create_dir_all(&dir).expect("create @types/node");
    assert!(_test_has_types_node(tmp.path()));
}

#[test]
fn has_types_node_true_for_hoisted_ancestor() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let workspace_root = tmp.path();
    let pkg = workspace_root.join("packages").join("inner");
    std::fs::create_dir_all(&pkg).expect("create inner pkg");
    std::fs::create_dir_all(workspace_root.join("node_modules/@types/node"))
        .expect("create hoisted @types/node");
    assert!(_test_has_types_node(&pkg));
}

#[test]
fn ecosystem_locate_returns_empty_when_types_node_present() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    std::fs::create_dir_all(tmp.path().join("node_modules/@types/node"))
        .expect("seed @types/node");

    use crate::ecosystem::{Ecosystem, EcosystemId};
    use std::collections::HashMap;
    let manifests: HashMap<EcosystemId, Vec<std::path::PathBuf>> = HashMap::new();
    let active: Vec<EcosystemId> = Vec::new();
    let ctx = LocateContext {
        project_root: tmp.path(),
        manifests: &manifests,
        active_ecosystems: &active,
    };
    let roots = <NodeBuiltinsEcosystem as Ecosystem>::locate_roots(
        &NodeBuiltinsEcosystem,
        &ctx,
    );
    assert!(roots.is_empty());
}

#[test]
fn ecosystem_locate_returns_synthetic_when_types_node_absent() {
    let tmp = tempfile::tempdir().expect("create tempdir");

    use crate::ecosystem::{Ecosystem, EcosystemId};
    use std::collections::HashMap;
    let manifests: HashMap<EcosystemId, Vec<std::path::PathBuf>> = HashMap::new();
    let active: Vec<EcosystemId> = Vec::new();
    let ctx = LocateContext {
        project_root: tmp.path(),
        manifests: &manifests,
        active_ecosystems: &active,
    };
    let roots = <NodeBuiltinsEcosystem as Ecosystem>::locate_roots(
        &NodeBuiltinsEcosystem,
        &ctx,
    );
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].module_path, "node-builtins");
}

#[test]
fn legacy_parse_metadata_only_short_circuits_when_types_node_present() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    std::fs::create_dir_all(tmp.path().join("node_modules/@types/node"))
        .expect("seed @types/node");
    let parsed =
        <NodeBuiltinsEcosystem as ExternalSourceLocator>::parse_metadata_only(
            &NodeBuiltinsEcosystem,
            tmp.path(),
        );
    assert!(parsed.is_none());
}

#[test]
fn legacy_parse_metadata_only_emits_when_types_node_absent() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let parsed =
        <NodeBuiltinsEcosystem as ExternalSourceLocator>::parse_metadata_only(
            &NodeBuiltinsEcosystem,
            tmp.path(),
        )
        .expect("synthetic should fire");
    assert_eq!(parsed.len(), 34);
}
