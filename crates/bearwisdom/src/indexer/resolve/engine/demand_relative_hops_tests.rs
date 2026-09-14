// =============================================================================
// engine/demand_relative_hops_tests — relative import and re-export hops join
// the frontier
// =============================================================================

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use crate::indexer::resolve::engine::testkit;
use crate::languages::typescript::profile::TYPESCRIPT_PROFILE;
use crate::types::{EdgeKind, ExtractedRef};

fn reexport_ref(target: &str, module: &str) -> ExtractedRef {
    let mut r = testkit::call_ref(target);
    r.kind = EdgeKind::Imports;
    r.is_reexport = true;
    r.module = Some(module.to_string());
    r
}

fn import_ref(target: &str, module: &str) -> ExtractedRef {
    let mut r = testkit::call_ref(target);
    r.kind = EdgeKind::Imports;
    r.is_import_binding = true;
    r.module = Some(module.to_string());
    r
}

/// `import './global';` — no binding, only the module it loads.
fn side_effect_import(module: &str) -> ExtractedRef {
    let mut r = testkit::call_ref(module);
    r.kind = EdgeKind::Imports;
    r.module = Some(module.to_string());
    r
}

/// `pkg/index.d.ts` forwarding to `./dist` (a directory module) and
/// `./baz` (a sibling file), beside a `./types` import that is not re-exported.
fn seed_package() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::TempDir::new().unwrap();
    let pkg = root.path().join("pkg");
    fs::create_dir_all(pkg.join("dist")).unwrap();
    fs::write(pkg.join("index.d.ts"), "export * from './dist';\n").unwrap();
    fs::write(pkg.join("dist").join("index.d.ts"), "export {};\n").unwrap();
    fs::write(
        pkg.join("baz.d.ts"),
        "export declare function Baz(): void;\n",
    )
    .unwrap();
    fs::write(pkg.join("types.d.ts"), "export type T = string;\n").unwrap();
    let entry = pkg.join("index.d.ts");
    (root, entry)
}

fn collect(entry: &std::path::Path, refs: &[ExtractedRef]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    super::collect(
        entry,
        "typescript",
        &TYPESCRIPT_PROFILE,
        refs,
        &mut seen,
        &mut out,
    );
    out
}

#[test]
fn relative_star_and_named_reexports_resolve_to_directory_index_and_sibling_file() {
    let (_root, entry) = seed_package();
    let refs = [reexport_ref("*", "./dist"), reexport_ref("Baz", "./baz")];
    let out = collect(&entry, &refs);
    let dir = entry.parent().unwrap();
    assert_eq!(
        out,
        vec![dir.join("dist").join("index.d.ts"), dir.join("baz.d.ts")],
        "a directory star must land on its index module and a named re-export on its sibling file"
    );
}

#[test]
fn relative_imports_and_side_effect_imports_are_hops_too() {
    let (_root, entry) = seed_package();
    let dir = entry.parent().unwrap();
    let refs = [import_ref("T", "./types"), side_effect_import("./dist")];
    assert_eq!(
        collect(&entry, &refs),
        vec![dir.join("types.d.ts"), dir.join("dist").join("index.d.ts")],
        "a declaration file's imports carry the types its surface mentions and the augmentations it loads"
    );
}

#[test]
fn bare_package_specifiers_and_body_calls_are_not_followed_here() {
    let (_root, entry) = seed_package();
    let refs = [
        reexport_ref("*", "other-pkg"),
        import_ref("x", "lodash"),
        testkit::call_ref("Baz"),
    ];
    assert!(
        collect(&entry, &refs).is_empty(),
        "a bare specifier belongs to the package-entry pull; a call names no module"
    );
}

#[test]
fn a_hop_already_seen_is_pushed_once() {
    let (_root, entry) = seed_package();
    let refs = [reexport_ref("*", "./dist"), reexport_ref("Foo", "./dist")];
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    super::collect(
        &entry,
        "typescript",
        &TYPESCRIPT_PROFILE,
        &refs,
        &mut seen,
        &mut out,
    );
    assert_eq!(out.len(), 1);
    super::collect(
        &entry,
        "typescript",
        &TYPESCRIPT_PROFILE,
        &refs,
        &mut seen,
        &mut out,
    );
    assert_eq!(out.len(), 1, "`seen` dedupes across waves");
}

#[test]
fn a_language_without_a_relative_module_policy_contributes_nothing() {
    let (_root, entry) = seed_package();
    let refs = [reexport_ref("*", "./dist")];
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    super::collect(
        &entry,
        "fortran",
        &TYPESCRIPT_PROFILE,
        &refs,
        &mut seen,
        &mut out,
    );
    assert!(
        out.is_empty(),
        "no ecosystem owns the language: fail closed"
    );
}
