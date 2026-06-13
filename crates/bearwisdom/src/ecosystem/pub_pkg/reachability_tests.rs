// ---------------------------------------------------------------------------
// Reachability tests — cross-package export-leaf follow
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{
    resolve_dart_package_entry, resolve_dart_package_entry_with_siblings, SiblingRoots,
};
use crate::ecosystem::externals::ExternalDepRoot;

fn mkdep(root: PathBuf, name: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: name.to_string(),
        version: String::new(),
        root,
        ecosystem: super::super::LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

/// Build a sibling-roots map (`module_path → lib_root`) from a dep slice,
/// mirroring what the demand pipeline does before walking.
fn siblings(deps: &[ExternalDepRoot]) -> SiblingRoots {
    let mut m: HashMap<String, PathBuf> = HashMap::new();
    for d in deps {
        m.insert(d.module_path.clone(), d.root.clone());
    }
    m
}

/// Write `lib/<file>` (creating parent dirs) under a package lib root.
fn write_lib(lib: &Path, rel: &str, body: &str) {
    let full = lib.join(rel);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, body).unwrap();
}

/// (a) Walking the framework entry package `test` follows the cross-package
/// `export 'package:matcher/expect.dart' show ...;` chain into the `matcher`
/// leaf and indexes the top-level `expect` FUNCTION file.
#[test]
fn cross_package_export_reaches_defining_leaf() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cache = tmp.path();

    // package `matcher`: secondary entry expect.dart re-exports the impl leaf
    // src/expect/expect.dart where the top-level `expect` function lives.
    let matcher_lib = cache.join("matcher-1.0.0").join("lib");
    write_lib(
        &matcher_lib,
        "matcher.dart",
        "export 'src/core_matchers.dart';\n",
    );
    write_lib(&matcher_lib, "src/core_matchers.dart", "class Matcher {}\n");
    write_lib(
        &matcher_lib,
        "expect.dart",
        "export 'src/expect/expect.dart' show expect, fail;\n",
    );
    write_lib(
        &matcher_lib,
        "src/expect/expect.dart",
        "void expect(dynamic actual, dynamic matcher) {}\n",
    );

    // package `test`: the framework entry re-exports the matcher leaf across
    // the package boundary.
    let test_lib = cache.join("test-1.0.0").join("lib");
    write_lib(
        &test_lib,
        "test.dart",
        "export 'package:matcher/expect.dart';\n",
    );

    let test_dep = mkdep(test_lib.clone(), "test");
    let matcher_dep = mkdep(matcher_lib.clone(), "matcher");
    let sibs = siblings(&[test_dep.clone(), matcher_dep]);

    let files = resolve_dart_package_entry_with_siblings(&test_dep, &sibs);
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

    // The leaf defining `expect` is reached and labeled under matcher's path.
    assert!(
        paths.contains(&"ext:dart:matcher/expect.dart"),
        "expected matcher entry, got {paths:?}"
    );
    assert!(
        paths.contains(&"ext:dart:matcher/src/expect/expect.dart"),
        "expected matcher expect leaf, got {paths:?}"
    );
    // The cross-package files are labeled with the OWNING package, not `test`.
    assert!(
        !paths.iter().any(|p| p.starts_with("ext:dart:test/src/expect")),
        "leaf must keep its owning-package label, got {paths:?}"
    );
}

/// (b) A re-export chain longer than `DART_EXPORT_MAX_DEPTH` stops — no
/// runaway. Build entry → a → b → c → d → e where each hops one package; the
/// walk must bottom out at the depth cap, not index the deepest leaf.
#[test]
fn cross_package_chain_bounded_by_depth_cap() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cache = tmp.path();

    // p0 entry → p1 → ... → p8_leaf. Each package re-exports the next across
    // a package boundary, one hop per level — a chain longer than the depth
    // cap, so the deepest leaves must be unreachable.
    let names = ["p0", "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8"];
    let mut deps = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let lib = cache.join(format!("{name}-1.0.0")).join("lib");
        if i + 1 < names.len() {
            let next = names[i + 1];
            write_lib(
                &lib,
                &format!("{name}.dart"),
                &format!("export 'package:{next}/{next}.dart';\n"),
            );
        } else {
            write_lib(
                &lib,
                &format!("{name}.dart"),
                "void deepLeafSymbol() {}\n",
            );
        }
        deps.push(mkdep(lib, name));
    }
    let sibs = siblings(&deps);

    let files = resolve_dart_package_entry_with_siblings(&deps[0], &sibs);
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

    // The entry plus a bounded number of hops are present; the deepest leaves
    // past the cap must NOT be reached.
    assert!(paths.contains(&"ext:dart:p0/p0.dart"));
    assert!(
        !paths.contains(&"ext:dart:p8/p8.dart"),
        "depth cap breached — deepest leaf reached: {paths:?}"
    );
    assert!(
        !paths.contains(&"ext:dart:p7/p7.dart"),
        "depth cap breached — second-deepest leaf reached: {paths:?}"
    );
}

/// (c) The walk is file-granular: a `show expect;` clause does NOT prune
/// other top-level files of the exported library. The whole exported file is
/// pulled; downstream symbol resolution filters by name. This test documents
/// that contract — both shown and unshown siblings of the exported FILE are
/// present once the file is reached, but only the exported file itself (not
/// arbitrary siblings) is followed.
#[test]
fn show_clause_is_file_granular_not_symbol_filtered() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cache = tmp.path();

    let dep_lib = cache.join("matcher-1.0.0").join("lib");
    // expect.dart shows only `expect` from the leaf, but the leaf file also
    // declares `fail`. File-granular walk pulls the whole leaf file.
    write_lib(
        &dep_lib,
        "matcher.dart",
        "export 'package:matcher/expect.dart' show expect;\n",
    );
    write_lib(
        &dep_lib,
        "expect.dart",
        "export 'src/leaf.dart';\n",
    );
    write_lib(
        &dep_lib,
        "src/leaf.dart",
        "void expect() {}\nvoid fail() {}\n",
    );
    // A sibling file NOT named in any export — must stay unwalked.
    write_lib(&dep_lib, "src/unreferenced.dart", "void orphan() {}\n");

    let dep = mkdep(dep_lib, "matcher");
    let sibs = siblings(&[dep.clone()]);
    let files = resolve_dart_package_entry_with_siblings(&dep, &sibs);
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

    assert!(paths.contains(&"ext:dart:matcher/src/leaf.dart"));
    assert!(
        !paths.iter().any(|p| p.ends_with("unreferenced.dart")),
        "non-exported sibling must not be walked: {paths:?}"
    );
}

/// (d) Regression: a package with no cross-package re-exports walks exactly
/// as the sibling-unaware path did — the sibling map is inert.
#[test]
fn no_reexport_package_walks_unchanged() {
    let tmp = tempfile::TempDir::new().unwrap();
    let lib = tmp.path().join("lib");
    std::fs::create_dir_all(lib.join("src")).unwrap();
    std::fs::write(
        lib.join("provider.dart"),
        "export 'src/internal.dart';\nexport 'public.dart';\n",
    )
    .unwrap();
    std::fs::write(lib.join("public.dart"), "class Public {}\n").unwrap();
    std::fs::write(lib.join("src").join("internal.dart"), "class Internal {}\n").unwrap();

    let dep = mkdep(lib.clone(), "provider");
    let sibs = siblings(&[dep.clone()]);

    let with_siblings = resolve_dart_package_entry_with_siblings(&dep, &sibs);
    let without_siblings = resolve_dart_package_entry(&dep);

    let mut a: Vec<String> = with_siblings
        .iter()
        .map(|f| f.relative_path.clone())
        .collect();
    let mut b: Vec<String> = without_siblings
        .iter()
        .map(|f| f.relative_path.clone())
        .collect();
    a.sort();
    b.sort();
    assert_eq!(a, b, "sibling-aware walk diverged on a no-reexport package");
    assert_eq!(a.len(), 3);
}

/// A cross-package `export` whose target package is NOT in the sibling map
/// (not discovered on disk) is skipped without error — the chain continues
/// for the in-package specs.
#[test]
fn unknown_sibling_package_is_skipped() {
    let tmp = tempfile::TempDir::new().unwrap();
    let lib = tmp.path().join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(
        lib.join("test.dart"),
        "export 'package:not_discovered/foo.dart';\nexport 'local.dart';\n",
    )
    .unwrap();
    std::fs::write(lib.join("local.dart"), "class Local {}\n").unwrap();

    let dep = mkdep(lib.clone(), "test");
    let sibs = siblings(&[dep.clone()]); // not_discovered absent on purpose
    let files = resolve_dart_package_entry_with_siblings(&dep, &sibs);
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();

    assert!(paths.contains(&"ext:dart:test/test.dart"));
    assert!(paths.contains(&"ext:dart:test/local.dart"));
    assert_eq!(paths.len(), 2);
}

/// A cross-package `import` is a consumer edge, not a re-export — the walk
/// must NOT hop into the imported package. Following imports would expand
/// into the import closure of the entire dependency graph.
#[test]
fn cross_package_import_does_not_hop() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cache = tmp.path();

    let other_lib = cache.join("other-1.0.0").join("lib");
    write_lib(&other_lib, "other.dart", "void helper() {}\n");

    let app_lib = cache.join("app-1.0.0").join("lib");
    write_lib(
        &app_lib,
        "app.dart",
        "import 'package:other/other.dart';\nvoid run() {}\n",
    );

    let app_dep = mkdep(app_lib.clone(), "app");
    let other_dep = mkdep(other_lib, "other");
    let sibs = siblings(&[app_dep.clone(), other_dep]);

    let files = resolve_dart_package_entry_with_siblings(&app_dep, &sibs);
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert!(
        paths.contains(&"ext:dart:app/app.dart"),
        "entry must be pulled, got {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.starts_with("ext:dart:other/")),
        "a cross-package import must not hop, got {paths:?}"
    );
}

/// A caller-owned `seen` set spans roots: a file reachable from two entry
/// packages is pulled exactly once across the whole pre-pull.
#[test]
fn shared_seen_pulls_each_file_once_across_roots() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cache = tmp.path();

    let shared_lib = cache.join("shared-1.0.0").join("lib");
    write_lib(&shared_lib, "shared.dart", "void common() {}\n");

    let a_lib = cache.join("a-1.0.0").join("lib");
    write_lib(&a_lib, "a.dart", "export 'package:shared/shared.dart';\n");
    let b_lib = cache.join("b-1.0.0").join("lib");
    write_lib(&b_lib, "b.dart", "export 'package:shared/shared.dart';\n");

    let a_dep = mkdep(a_lib, "a");
    let b_dep = mkdep(b_lib, "b");
    let shared_dep = mkdep(shared_lib, "shared");
    let sibs = siblings(&[a_dep.clone(), b_dep.clone(), shared_dep]);

    let mut seen = std::collections::HashSet::new();
    let mut all = super::resolve_dart_package_entry_shared_seen(&a_dep, &sibs, &mut seen);
    all.extend(super::resolve_dart_package_entry_shared_seen(
        &b_dep, &sibs, &mut seen,
    ));
    let shared_pulls = all
        .iter()
        .filter(|f| f.relative_path == "ext:dart:shared/shared.dart")
        .count();
    assert_eq!(
        shared_pulls, 1,
        "shared leaf must be pulled once across roots, got {all:?}"
    );
}
