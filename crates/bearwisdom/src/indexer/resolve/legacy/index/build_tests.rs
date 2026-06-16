use std::collections::HashMap;

use super::{_test_resolve_workspace_pkg_entry, _test_snapshot_path_aliases};
use crate::ecosystem::manifest::{ManifestData, ManifestKind};
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

// ---------------------------------------------------------------------------
// snapshot_path_aliases — per-package keying + union-fallback prefix
// ---------------------------------------------------------------------------

/// A package manifest entry whose NPM data declares `aliases` as
/// `(prefix, target)` pairs.
fn npm_pkg(
    id: i64,
    aliases: &[(&str, &str)],
) -> (i64, HashMap<ManifestKind, ManifestData>) {
    let mut data = ManifestData::default();
    data.path_aliases = aliases
        .iter()
        .map(|(a, t)| (a.to_string(), t.to_string()))
        .collect();
    let mut m = HashMap::new();
    m.insert(ManifestKind::Npm, data);
    (id, m)
}

/// A package manifest entry with no NPM aliases (no path_aliases declared).
fn empty_pkg(id: i64) -> (i64, HashMap<ManifestKind, ManifestData>) {
    (id, HashMap::new())
}

fn pkg_dirs(entries: &[(i64, &str)]) -> HashMap<i64, String> {
    entries.iter().map(|(id, p)| (*id, p.to_string())).collect()
}

/// Look up a single (alias, target) entry for a package id.
fn alias_for<'a>(
    map: &'a rustc_hash::FxHashMap<i64, Vec<(String, String)>>,
    pkg_id: i64,
    alias: &str,
) -> Option<&'a str> {
    map.get(&pkg_id)?
        .iter()
        .find(|(a, _)| a == alias)
        .map(|(_, t)| t.as_str())
}

#[test]
fn per_package_aliases_keyed_separately_not_cross_keyed() {
    // Two workspace packages each declare `"@/*": ["src/*"]` in their own
    // tsconfig. Each package's alias must rewrite under its OWN directory.
    let by_package: HashMap<i64, HashMap<ManifestKind, ManifestData>> = [
        npm_pkg(1, &[("@/", "src/")]),
        npm_pkg(2, &[("@/", "src/")]),
    ]
    .into_iter()
    .collect();
    let dirs = pkg_dirs(&[(1, "apps/a"), (2, "apps/b")]);

    let by_pkg = _test_snapshot_path_aliases(&[], &by_package, &dirs);

    // Each `@/` rewrites under its own package dir — never cross-keyed.
    assert_eq!(alias_for(&by_pkg, 1, "@/"), Some("apps/a/src/"));
    assert_eq!(alias_for(&by_pkg, 2, "@/"), Some("apps/b/src/"));
}

#[test]
fn package_without_own_aliases_inherits_root_union_prefixed() {
    // Package declares no own aliases; the root tsconfig declares
    // `"@/*": ["src/*"]`. The inherited union target must be prefixed with
    // the package's directory so it points at package-relative files.
    let union = vec![("@/".to_string(), "src/".to_string())];
    let by_package: HashMap<i64, HashMap<ManifestKind, ManifestData>> =
        [empty_pkg(5)].into_iter().collect();
    let dirs = pkg_dirs(&[(5, "apps/c")]);

    let by_pkg = _test_snapshot_path_aliases(&union, &by_package, &dirs);

    assert_eq!(alias_for(&by_pkg, 5, "@/"), Some("apps/c/src/"));
}

#[test]
fn package_without_dir_omitted_from_per_package_map() {
    // No own aliases AND no known directory → not keyed; the file falls
    // through to the raw union at lookup time (verified by the absence of an
    // entry, since the union itself is never stored here).
    let union = vec![("@/".to_string(), "src/".to_string())];
    let by_package: HashMap<i64, HashMap<ManifestKind, ManifestData>> =
        [empty_pkg(9)].into_iter().collect();
    let dirs = pkg_dirs(&[]);

    let by_pkg = _test_snapshot_path_aliases(&union, &by_package, &dirs);

    assert!(by_pkg.get(&9).is_none());
}
