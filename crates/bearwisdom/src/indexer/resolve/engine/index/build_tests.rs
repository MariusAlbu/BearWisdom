use std::collections::HashMap;

use super::_test_resolve_workspace_pkg_entry;
use crate::indexer::module_resolution::FilePathIndex;

fn declared(name: &str, id: i64) -> HashMap<String, i64> {
    let mut m = HashMap::new();
    m.insert(name.to_string(), id);
    m
}

fn paths(id: i64, root: &str) -> HashMap<i64, String> {
    let mut m = HashMap::new();
    m.insert(id, root.to_string());
    m
}

#[test]
fn monorepo_workspace_specifier_resolves_to_entry_file() {
    let declared = declared("@myorg/ui", 7);
    let paths = paths(7, "packages/ui");
    let files = &["packages/ui/src/index.ts", "packages/ui/src/button.ts"];
    let index = FilePathIndex::build(files);

    assert_eq!(
        _test_resolve_workspace_pkg_entry("@myorg/ui", &declared, &paths, &index),
        Some("packages/ui/src/index.ts".to_string())
    );
}

#[test]
fn root_index_preferred_when_present() {
    let declared = declared("@myorg/ui", 7);
    let paths = paths(7, "packages/ui");
    let files = &["packages/ui/index.ts", "packages/ui/src/index.ts"];
    let index = FilePathIndex::build(files);

    // `index.*` at the package root wins over `src/index.*`.
    assert_eq!(
        _test_resolve_workspace_pkg_entry("@myorg/ui", &declared, &paths, &index),
        Some("packages/ui/index.ts".to_string())
    );
}

#[test]
fn deep_import_strips_subpath_to_package_name() {
    let declared = declared("@myorg/ui", 7);
    let paths = paths(7, "packages/ui");
    let files = &["packages/ui/src/index.ts"];
    let index = FilePathIndex::build(files);

    assert_eq!(
        _test_resolve_workspace_pkg_entry("@myorg/ui/button", &declared, &paths, &index),
        Some("packages/ui/src/index.ts".to_string())
    );
}

#[test]
fn unknown_specifier_resolves_to_none() {
    let declared = declared("@myorg/ui", 7);
    let paths = paths(7, "packages/ui");
    let files = &["packages/ui/src/index.ts"];
    let index = FilePathIndex::build(files);

    assert_eq!(
        _test_resolve_workspace_pkg_entry("@other/x", &declared, &paths, &index),
        None
    );
}

#[test]
fn known_package_without_entry_file_resolves_to_none() {
    let declared = declared("@myorg/ui", 7);
    let paths = paths(7, "packages/ui");
    // No index.* / src/index.* under the package root.
    let files = &["packages/ui/src/button.ts"];
    let index = FilePathIndex::build(files);

    assert_eq!(
        _test_resolve_workspace_pkg_entry("@myorg/ui", &declared, &paths, &index),
        None
    );
}

#[test]
fn empty_root_probes_bare_index() {
    let declared = declared("ui", 7);
    let paths = paths(7, "");
    let files = &["index.ts"];
    let index = FilePathIndex::build(files);

    assert_eq!(
        _test_resolve_workspace_pkg_entry("ui", &declared, &paths, &index),
        Some("index.ts".to_string())
    );
}

#[test]
fn tsx_entry_extension_resolves() {
    let declared = declared("@myorg/ui", 7);
    let paths = paths(7, "packages/ui");
    let files = &["packages/ui/src/index.tsx"];
    let index = FilePathIndex::build(files);

    assert_eq!(
        _test_resolve_workspace_pkg_entry("@myorg/ui", &declared, &paths, &index),
        Some("packages/ui/src/index.tsx".to_string())
    );
}
