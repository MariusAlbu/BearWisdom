//! Integration test for A4 — cross-package edge via workspace declared_name.
//!
//! Builds a pnpm-style two-package workspace on disk, runs the full indexer,
//! and asserts that `import { formatDate } from '@myorg/utils'` in the app
//! package produces a confidence-1.0 edge pointing at the producer symbol
//! via the `ts_workspace_pkg` resolver path.

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

fn build_pnpm_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_file(
        root,
        "package.json",
        r#"{"name":"monorepo","private":true,"workspaces":["packages/*"]}"#,
    );
    write_file(root, "pnpm-workspace.yaml", "packages:\n  - \"packages/*\"\n");

    write_file(
        root,
        "packages/utils/package.json",
        r#"{"name":"@myorg/utils","version":"0.1.0","main":"src/index.ts"}"#,
    );
    write_file(
        root,
        "packages/utils/src/index.ts",
        r#"export function formatDate(d: Date): string {
    return d.toISOString();
}
"#,
    );

    write_file(
        root,
        "packages/app/package.json",
        r#"{"name":"@myorg/app","version":"0.1.0","dependencies":{"@myorg/utils":"workspace:*"}}"#,
    );
    write_file(
        root,
        "packages/app/src/main.ts",
        r#"import { formatDate } from '@myorg/utils';

export function main(): string {
    return formatDate(new Date());
}
"#,
    );

    tmp
}

#[test]
fn cross_package_import_resolves_at_confidence_1() {
    let tmp = build_pnpm_workspace();
    let root = tmp.path();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // The producer symbol — `formatDate` declared in @myorg/utils.
    let producer_id: i64 = db
        .query_row(
            "SELECT s.id FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE s.name = 'formatDate' AND f.path LIKE '%utils%'",
            [],
            |row| row.get(0),
        )
        .expect("producer symbol formatDate not indexed");

    // The consumer symbol — `main` in @myorg/app — which references formatDate.
    let consumer_id: i64 = db
        .query_row(
            "SELECT s.id FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE s.name = 'main' AND f.path LIKE '%app%'",
            [],
            |row| row.get(0),
        )
        .expect("consumer symbol main not indexed");

    // An edge must exist from main → formatDate at confidence 1.0.
    let edge_confidence: Option<f64> = db
        .query_row(
            "SELECT confidence FROM edges
             WHERE source_id = ?1 AND target_id = ?2
             ORDER BY confidence DESC LIMIT 1",
            rusqlite::params![consumer_id, producer_id],
            |row| row.get(0),
        )
        .ok();

    let confidence = edge_confidence.unwrap_or_else(|| {
        // On failure, dump all edges out of consumer so the diagnostic
        // shows whether the ref resolved elsewhere or didn't resolve.
        let mut stmt = db
            .prepare(
                "SELECT e.target_id, e.kind, e.confidence, s2.name, f2.path
                 FROM edges e
                 LEFT JOIN symbols s2 ON s2.id = e.target_id
                 LEFT JOIN files f2 ON f2.id = s2.file_id
                 WHERE e.source_id = ?1",
            )
            .unwrap();
        let dump: Vec<(i64, String, f64, Option<String>, Option<String>)> = stmt
            .query_map(rusqlite::params![consumer_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .unwrap()
            .flatten()
            .collect();
        panic!(
            "no edge from main (id={consumer_id}) → formatDate (id={producer_id}); \
             consumer edges: {dump:?}"
        );
    });

    assert!(
        confidence >= 0.999,
        "cross-package edge should resolve at confidence 1.0, got {confidence}"
    );

    // Packages must both exist, keyed by declared_name — sanity check that
    // the workspace detection picked them up.
    let declared_names: Vec<String> = db
        .prepare("SELECT declared_name FROM packages WHERE declared_name IS NOT NULL ORDER BY declared_name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(
        declared_names,
        vec!["@myorg/app".to_string(), "@myorg/utils".to_string()],
        "expected both workspace packages detected"
    );
}
