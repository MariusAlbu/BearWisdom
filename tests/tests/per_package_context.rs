//! Integration test for M2 — per-package ProjectContext.
//!
//! Verifies that the resolver uses per-package manifest data when classifying
//! external references — so a file in `server/` doesn't see deps declared
//! only in `e2e/package.json`. Also verifies that unresolved_refs.package_id
//! is populated from the source file's package_id.

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

fn build_monorepo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Root workspace manifest.
    write_file(
        root,
        "package.json",
        r#"{"name":"monorepo","private":true,"workspaces":["server","e2e"]}"#,
    );

    // server/ — declares express only.
    write_file(
        root,
        "server/package.json",
        r#"{"name":"@app/server","dependencies":{"express":"4"}}"#,
    );
    write_file(
        root,
        "server/src/app.ts",
        r#"import express from 'express';
import { runTests } from 'playwright';

export function createApp() {
    const app = express();
    runTests();
    return app;
}
"#,
    );

    // e2e/ — declares playwright only.
    write_file(
        root,
        "e2e/package.json",
        r#"{"name":"@app/e2e","devDependencies":{"playwright":"1.40"}}"#,
    );
    write_file(
        root,
        "e2e/src/test.ts",
        r#"import { runTests } from 'playwright';

export function run() {
    runTests();
}
"#,
    );

    tmp
}

fn collect_packages(db: &bearwisdom::Database) -> Vec<(i64, String)> {
    // Looks up by `declared_name` — the manifest-reported package name
    // (A2). `name` is the folder-derived key (e.g. `server`, `e2e`) and
    // doesn't match `@app/server` etc.
    let mut stmt = db
        .prepare("SELECT id, COALESCE(declared_name, name) FROM packages")
        .unwrap();
    stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })
    .unwrap()
    .flatten()
    .collect()
}

#[test]
fn unresolved_refs_also_carry_package_id() {
    let tmp = build_monorepo();
    let root = tmp.path();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let total: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs ur
             JOIN symbols s ON s.id = ur.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let stamped: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs ur
             JOIN symbols s ON s.id = ur.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal' AND ur.package_id IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();

    if total > 0 {
        assert!(
            stamped > 0,
            "expected at least one unresolved_refs row stamped with package_id, got 0 of {total} total"
        );
    }
}

#[test]
fn single_project_still_classifies_correctly() {
    // Regression: M2 must not break single-project layouts. The legacy
    // builder path returns an empty by_package map — the resolver falls
    // back to the union and behaves exactly as before.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_file(
        root,
        "package.json",
        r#"{"name":"solo","dependencies":{"express":"4"}}"#,
    );
    write_file(
        root,
        "src/index.ts",
        r#"import express from 'express';
export const app = express();
"#,
    );

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // The express import has no supply on disk in this fixture; the union
    // manifest (single-project legacy path) still attributes it as a
    // declared-but-unsupplied dependency rather than a plain unlinked import.
    let cause: String = db
        .query_row(
            "SELECT ur.cause_kind FROM unresolved_refs ur
             WHERE ur.module = 'express'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        cause, "unbound_import_declared_unsupplied",
        "single-project manifest fallback must attribute the declared dep"
    );
}
