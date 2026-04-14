//! Integration test for M2 — per-package ProjectContext.
//!
//! Verifies that the resolver uses per-package manifest data when classifying
//! external references — so a file in `server/` doesn't see deps declared
//! only in `e2e/package.json`. Also verifies that external_refs.package_id
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
    let mut stmt = db.prepare("SELECT id, name FROM packages").unwrap();
    stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })
    .unwrap()
    .flatten()
    .collect()
}

#[test]
fn external_refs_are_scoped_per_package() {
    let tmp = build_monorepo();
    let root = tmp.path();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let packages = collect_packages(&db);
    let server_id = packages
        .iter()
        .find(|(_, n)| n == "@app/server")
        .map(|(id, _)| *id)
        .expect("@app/server package not in DB");
    let e2e_id = packages
        .iter()
        .find(|(_, n)| n == "@app/e2e")
        .map(|(id, _)| *id)
        .expect("@app/e2e package not in DB");

    // Every internal external_refs row must have a package_id that matches
    // the source file's package.
    let mut stmt = db
        .prepare(
            "SELECT er.namespace, er.package_id, f.path
             FROM external_refs er
             JOIN symbols s ON s.id = er.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal'",
        )
        .unwrap();
    let rows: Vec<(String, Option<i64>, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .flatten()
        .collect();

    assert!(
        !rows.is_empty(),
        "expected some external_refs rows; got none"
    );

    let mut found_pkg_id = false;
    for (ns, pkg_id, path) in &rows {
        let normalized = path.replace('\\', "/");
        if normalized.starts_with("server/") {
            assert_eq!(
                *pkg_id,
                Some(server_id),
                "server/ ref ('{ns}' in {path}) has wrong package_id {pkg_id:?}"
            );
            found_pkg_id = true;
        } else if normalized.starts_with("e2e/") {
            assert_eq!(
                *pkg_id,
                Some(e2e_id),
                "e2e/ ref ('{ns}' in {path}) has wrong package_id {pkg_id:?}"
            );
            found_pkg_id = true;
        }
    }
    assert!(
        found_pkg_id,
        "expected at least one external_ref tagged with a server or e2e package_id"
    );

    // Per-package classification invariant: when the resolver chose an
    // `express` namespace via non-import classification, it must have come
    // from a server/ file (server declared it).
    //
    // Note: import statements (kind='imports') generate a ref with their
    // module path as the namespace regardless of manifest — `import X from 'Y'`
    // IS a real reference to external module Y even if Y is missing from
    // package.json (that's a user code bug, not a classification bug).
    // So we only check non-import refs here.
    let mut stmt = db
        .prepare(
            "SELECT er.namespace, f.path
             FROM external_refs er
             JOIN symbols s ON s.id = er.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal' AND er.kind != 'imports'",
        )
        .unwrap();
    let non_import_rows: Vec<(String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .flatten()
        .collect();

    for (ns, path) in &non_import_rows {
        let normalized = path.replace('\\', "/");
        if ns == "express" {
            assert!(
                normalized.starts_with("server/"),
                "non-import express classified in non-server file: {path}"
            );
        }
        // playwright is legitimately imported in server (a declared bug) so
        // bare-name classification MIGHT fire — but with M2 per-package
        // isolation, server/ should no longer resolve bare `runTests` as
        // external via the playwright import (because server doesn't
        // declare playwright). Verify that:
        if ns == "playwright" && normalized.starts_with("server/") {
            // Debug: dump the offending row details.
            let details: Vec<(String, String, String)> = db
                .prepare(
                    "SELECT er.target_name, er.kind, f.path
                     FROM external_refs er
                     JOIN symbols s ON s.id = er.source_id
                     JOIN files   f ON f.id = s.file_id
                     WHERE er.namespace = 'playwright' AND f.path LIKE 'server%'",
                )
                .unwrap()
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .unwrap()
                .flatten()
                .collect();
            panic!(
                "M2 regression: bare-name 'playwright' classified in server/ file ({path}) — \
                 server does not declare playwright in its package.json. Details: {details:?}"
            );
        }
    }
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

    let external_count: i64 = db
        .query_row("SELECT COUNT(*) FROM external_refs", [], |row| row.get(0))
        .unwrap();
    assert!(
        external_count > 0,
        "single-project index should produce external_refs rows"
    );
}
