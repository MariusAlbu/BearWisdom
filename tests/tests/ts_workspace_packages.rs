//! A monorepo import of a sibling workspace package binds to that package's
//! own source: the manifest's entry candidates are tried in declared order
//! against the indexed files, so a build output that was never indexed yields
//! to the source entry a custom export condition names.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn seed_monorepo() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "package.json",
        r#"{"name":"monorepo","private":true,"workspaces":["packages/*"]}"#,
    );
    project.add_file(
        "packages/test-utils/package.json",
        r#"{"name":"@acme/test-utils","version":"0.0.0","main":"src/index.ts","types":"src/index.ts","exports":{".":{"types":"./src/index.ts","default":"./src/index.ts"}}}"#,
    );
    project.add_file(
        "packages/test-utils/src/index.ts",
        "export { sleep } from './sleep';\nexport { queryKey } from './queryKey';\n",
    );
    project.add_file(
        "packages/test-utils/src/sleep.ts",
        "export function sleep(ms: number): Promise<void> {\n  return new Promise((resolve) => setTimeout(resolve, ms));\n}\n",
    );
    project.add_file(
        "packages/test-utils/src/queryKey.ts",
        "export function queryKey(): string[] {\n  return ['k'];\n}\n",
    );
    project.add_file(
        "packages/core/package.json",
        r#"{"name":"@acme/core","version":"0.0.0","main":"build/legacy/index.cjs","types":"build/legacy/index.d.ts","exports":{".":{"@acme/custom-condition":"./src/index.ts","import":{"types":"./build/modern/index.d.ts","default":"./build/modern/index.js"}}},"devDependencies":{"@acme/test-utils":"workspace:*"}}"#,
    );
    project.add_file(
        "packages/core/src/index.ts",
        "export class QueryClient {\n  clear(): void {}\n}\n",
    );
    project.add_file(
        "packages/core/src/__tests__/client.test.ts",
        "import { sleep, queryKey } from '@acme/test-utils';\nimport { QueryClient } from '@acme/core';\n\nexport async function scenario(): Promise<void> {\n  const client = new QueryClient();\n  client.clear();\n  await sleep(10);\n  queryKey();\n}\n",
    );
    project
}

fn cross_package_edges(db: &bearwisdom::Database) -> Vec<(String, String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT s.name, e.kind, t.name, f.path FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files f ON f.id = t.file_id
             WHERE f.path LIKE 'packages/%' AND f.path NOT LIKE '%__tests__%'
             ORDER BY s.name, e.kind, t.name",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn edge(source: &str, kind: &str, target: &str, file: &str) -> (String, String, String, String) {
    (source.into(), kind.into(), target.into(), file.into())
}

#[test]
fn sibling_workspace_packages_bind_through_their_declared_source_entries() {
    let project = seed_monorepo();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let found = cross_package_edges(&db);
    for expected in [
        edge("scenario", "calls", "sleep", "packages/test-utils/src/sleep.ts"),
        edge("scenario", "calls", "queryKey", "packages/test-utils/src/queryKey.ts"),
        edge("scenario", "instantiates", "QueryClient", "packages/core/src/index.ts"),
        edge("scenario", "calls", "clear", "packages/core/src/index.ts"),
    ] {
        assert!(found.contains(&expected), "missing {expected:?} in {found:?}");
    }

    let unlinked: Vec<(String, String, Option<String>)> = {
        let mut stmt = db
            .prepare(
                "SELECT target_name, kind, module FROM unresolved_refs
                 WHERE cause_kind = 'unbound_import_unlinked' ORDER BY target_name",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert!(unlinked.is_empty(), "every workspace import links: {unlinked:?}");
}
