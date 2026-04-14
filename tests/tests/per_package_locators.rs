//! Integration tests for M3 — per-package locator scoping.
//!
//! Verifies that in a monorepo:
//!   1. `package_deps` is populated from every workspace package's manifest.
//!   2. A dep shared across packages (declared in both) is walked exactly
//!      once — no duplicate external files in the index.
//!   3. The scoped TS locator finds hoisted deps via ancestor walk when
//!      each package has no local node_modules.
//!   4. Single-project layouts continue to produce external_refs rows
//!      (no regression — M3 walks remain enabled when packages is empty).

use std::fs;
use std::path::Path;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(full, content).unwrap();
}

/// Build a monorepo with two packages that share a hoisted `react` and
/// one unique dep each. Writes a fake `node_modules/react/index.d.ts`
/// at the workspace root so the TS locator has something to walk.
fn build_hoisted_monorepo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_file(
        root,
        "package.json",
        r#"{"name":"ws","private":true,"workspaces":["apps/web","apps/server"]}"#,
    );

    write_file(
        root,
        "apps/web/package.json",
        r#"{"name":"web","dependencies":{"react":"18","axios":"1"}}"#,
    );
    write_file(
        root,
        "apps/web/src/index.tsx",
        r#"import { Component } from 'react';
export const App = Component;
"#,
    );

    write_file(
        root,
        "apps/server/package.json",
        r#"{"name":"server","dependencies":{"react":"18","express":"4"}}"#,
    );
    write_file(
        root,
        "apps/server/src/index.ts",
        r#"import { Component } from 'react';
export const render = Component;
"#,
    );

    // Hoisted node_modules at workspace root — both packages should find
    // react via ancestor walk.
    write_file(
        root,
        "node_modules/react/package.json",
        r#"{"name":"react","version":"18.2.0"}"#,
    );
    write_file(
        root,
        "node_modules/react/index.d.ts",
        r#"export declare function Component(): any;
export declare function useState<T>(initial: T): [T, (v: T) => void];
"#,
    );

    tmp
}

#[test]
fn shared_dep_walked_exactly_once() {
    // Two packages both declare react; hoisted node_modules/react/ at
    // workspace root. Dedup by (ecosystem, module_path, version) must
    // ensure react is walked exactly once — external file count from
    // react should match the number of .d.ts files under node_modules/react.
    let tmp = build_hoisted_monorepo();
    let mut db = TestProject::in_memory_db();

    // Ensure env doesn't leak from a prior run.
    std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");

    full_index(&mut db, tmp.path(), None, None, None).expect("index failed");

    let react_file_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM files WHERE origin = 'external' AND path LIKE 'ext:ts:react/%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        react_file_count, 1,
        "expected exactly one external file for react/index.d.ts, got {react_file_count}"
    );
}

#[test]
fn package_deps_populated_from_every_package() {
    // Acceptance criterion #5: a workspace with N packages must produce
    // one package_deps row per (package, dep). Shared deps produce ONE
    // row per declaring package (not a junction pivot).
    let tmp = build_hoisted_monorepo();
    let mut db = TestProject::in_memory_db();
    std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");
    full_index(&mut db, tmp.path(), None, None, None).expect("index failed");

    // Both packages declare react; only web declares axios; only server
    // declares express.
    let react_declarers: Vec<String> = db
        .prepare(
            "SELECT p.name FROM package_deps pd
             JOIN packages p ON p.id = pd.package_id
             WHERE pd.dep_name = 'react' AND pd.ecosystem = 'typescript'
             ORDER BY p.name",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(react_declarers, vec!["server", "web"]);

    let axios_declarers: Vec<String> = db
        .prepare(
            "SELECT p.name FROM package_deps pd
             JOIN packages p ON p.id = pd.package_id
             WHERE pd.dep_name = 'axios'
             ORDER BY p.name",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(axios_declarers, vec!["web"]);

    let express_declarers: Vec<String> = db
        .prepare(
            "SELECT p.name FROM package_deps pd
             JOIN packages p ON p.id = pd.package_id
             WHERE pd.dep_name = 'express'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(express_declarers, vec!["server"]);
}

#[test]
fn hoisted_externals_reachable_from_per_package_walk() {
    // With hoisted react at the workspace root and no per-package
    // node_modules, the M3 ancestor-walking TS locator must still find
    // react and index its symbols. This guards against a regression
    // where scoping to package_abs_path stopped probing upwards.
    let tmp = build_hoisted_monorepo();
    let mut db = TestProject::in_memory_db();
    std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");
    full_index(&mut db, tmp.path(), None, None, None).expect("index failed");

    let react_symbol_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols
             WHERE origin = 'external' AND qualified_name LIKE 'react.%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        react_symbol_count > 0,
        "expected react.* external symbols from hoisted node_modules, got {react_symbol_count}"
    );
}

#[test]
fn single_project_external_refs_still_work() {
    // Regression: single-project layouts (no packages detected) still
    // produce external_refs rows. M3 must not break the legacy path.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_file(
        root,
        "package.json",
        r#"{"name":"solo","dependencies":{"react":"18"}}"#,
    );
    write_file(
        root,
        "src/index.ts",
        r#"import { Component } from 'react';
export const X = Component;
"#,
    );
    write_file(
        root,
        "node_modules/react/package.json",
        r#"{"name":"react"}"#,
    );
    write_file(
        root,
        "node_modules/react/index.d.ts",
        r#"export declare function Component(): any;"#,
    );

    let mut db = TestProject::in_memory_db();
    std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");
    full_index(&mut db, root, None, None, None).expect("index failed");

    let react_symbol_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols
             WHERE origin = 'external' AND qualified_name LIKE 'react.%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        react_symbol_count > 0,
        "single-project must still index react externals, got {react_symbol_count}"
    );

    // No packages → no package_deps rows.
    let pd_count: i64 = db
        .query_row("SELECT COUNT(*) FROM package_deps", [], |r| r.get(0))
        .unwrap();
    assert_eq!(pd_count, 0, "single-project should not populate package_deps");
}
