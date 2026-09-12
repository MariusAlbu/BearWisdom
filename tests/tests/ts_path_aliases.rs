//! tsconfig `paths` aliases bind every position — exact aliases
//! (`"next-test-utils": [...]`), wildcard aliases (`"e2e-utils/*"`) and the
//! `@/*` convention — for calls, constructions and type annotations alike.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn seed_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "package.json",
        r#"{"name":"path-aliases","version":"0.0.1"}"#,
    );
    project.add_file(
        "tsconfig.json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"next-test-utils":["./test/lib/next-test-utils"],"e2e-utils/*":["./test/lib/e2e-utils/*"],"@/*":["./src/*"]}}}"#,
    );
    project.add_file(
        "test/lib/next-test-utils.ts",
        "export function retry(n: number): number {\n  return n;\n}\nexport class Runner {\n  go(): void {}\n}\n",
    );
    project.add_file(
        "test/lib/e2e-utils/ppr.ts",
        "export function ppr(): string {\n  return '';\n}\n",
    );
    project.add_file(
        "src/app/helper.ts",
        "export const helper = (): number => 1;\n",
    );
    project.add_file(
        "src/app/spec.ts",
        "import { retry, Runner } from 'next-test-utils';\nimport { ppr } from 'e2e-utils/ppr';\nimport { helper } from '@/app/helper';\n\nexport class Spec {\n  runner: Runner;\n  run(): void {\n    retry(1);\n    ppr();\n    helper();\n    new Runner().go();\n  }\n}\n",
    );
    project
}

fn cross_file_edges(db: &bearwisdom::Database) -> Vec<(String, String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT s.name, e.kind, t.name, f.path FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files f ON f.id = t.file_id
             WHERE f.path != (SELECT f2.path FROM symbols s2 JOIN files f2 ON f2.id = s2.file_id WHERE s2.id = e.source_id)
               AND f.path NOT LIKE 'ext:%'
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
fn exact_wildcard_and_at_aliases_bind_calls_constructions_and_types() {
    let project = seed_project();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let utils = "test/lib/next-test-utils.ts";
    let found = cross_file_edges(&db);
    for expected in [
        edge("Spec", "type_ref", "Runner", utils),
        edge("run", "calls", "retry", utils),
        edge("run", "instantiates", "Runner", utils),
        edge("run", "calls", "go", utils),
        edge("run", "calls", "ppr", "test/lib/e2e-utils/ppr.ts"),
        edge("run", "calls", "helper", "src/app/helper.ts"),
    ] {
        assert!(found.contains(&expected), "missing {expected:?} in {found:?}");
    }

    let unresolved: Vec<(String, String, Option<String>)> = {
        let mut stmt = db
            .prepare("SELECT target_name, kind, cause_kind FROM unresolved_refs ORDER BY target_name")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert!(unresolved.is_empty(), "every aliased import links: {unresolved:?}");
}
