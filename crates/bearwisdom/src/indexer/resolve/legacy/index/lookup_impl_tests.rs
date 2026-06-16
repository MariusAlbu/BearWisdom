use std::collections::HashMap;

use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{SymbolIndex, SymbolLookup};

/// Single-entry NPM manifest declaring the given `(prefix, target)` aliases.
fn npm_manifest(aliases: &[(&str, &str)]) -> HashMap<ManifestKind, ManifestData> {
    let mut data = ManifestData::default();
    data.path_aliases = aliases
        .iter()
        .map(|(a, t)| (a.to_string(), t.to_string()))
        .collect();
    let mut m = HashMap::new();
    m.insert(ManifestKind::Npm, data);
    m
}

/// A `ProjectContext` carrying a root-manifest alias union, per-package
/// manifests, and package directories — the inputs the alias snapshot reads.
fn ctx_with(
    root_aliases: &[(&str, &str)],
    by_package: HashMap<i64, HashMap<ManifestKind, ManifestData>>,
    pkg_dirs: &[(i64, &str)],
) -> ProjectContext {
    ProjectContext {
        manifests: npm_manifest(root_aliases),
        by_package,
        workspace_pkg_paths: pkg_dirs.iter().map(|(id, p)| (*id, p.to_string())).collect(),
        ..Default::default()
    }
}

#[test]
fn package_without_own_aliases_inherits_root_union_prefixed() {
    // Package 5 declares no own tsconfig aliases; the root tsconfig declares
    // `"@/*": ["src/*"]`. A ref in package 5 must rewrite under apps/c/src.
    let ctx = ctx_with(&[("@/", "src/")], HashMap::new(), &[(5, "apps/c")]);
    let index = SymbolIndex::build_with_context(&[], &HashMap::new(), Some(&ctx));

    assert_eq!(
        index.resolve_path_alias(Some(5), "@/y"),
        Some("apps/c/src/y".to_string())
    );
}

#[test]
fn no_package_file_gets_raw_union_unprefixed() {
    // A file with no package_id reads the raw root union — no directory
    // prefix, since there is no owning package to anchor it.
    let ctx = ctx_with(&[("@/", "src/")], HashMap::new(), &[(5, "apps/c")]);
    let index = SymbolIndex::build_with_context(&[], &HashMap::new(), Some(&ctx));

    assert_eq!(
        index.resolve_path_alias(None, "@/y"),
        Some("src/y".to_string())
    );
}

#[test]
fn per_package_aliases_resolve_under_own_directory() {
    // Two packages each declare `"@/*": ["src/*"]`; each rewrites under its
    // own directory, not cross-keyed.
    let mut by_package = HashMap::new();
    by_package.insert(1, npm_manifest(&[("@/", "src/")]));
    by_package.insert(2, npm_manifest(&[("@/", "src/")]));
    let ctx = ctx_with(&[], by_package, &[(1, "apps/a"), (2, "apps/b")]);
    let index = SymbolIndex::build_with_context(&[], &HashMap::new(), Some(&ctx));

    assert_eq!(
        index.resolve_path_alias(Some(1), "@/x"),
        Some("apps/a/src/x".to_string())
    );
    assert_eq!(
        index.resolve_path_alias(Some(2), "@/x"),
        Some("apps/b/src/x".to_string())
    );
}
