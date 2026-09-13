//! A literal `declare module "X"` block is the module named X: a named import
//! of X reaches the declaration's own exports, an `export =`-assigned value
//! publishes its declared type's members, and a project-owned ambient shim
//! supplies a package that ships no types. A default import of the assigned
//! value binds to that value; a member CALL on it is a qualified call site,
//! which only a configured program selects, so `p.dirname(...)` is not
//! asserted here.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn seed_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "package.json",
        r#"{"name":"declared-module-unit","version":"0.0.1","dependencies":{"shim-less-pkg":"^1.0.0"}}"#,
    );
    project.add_file(
        "node_modules/@types/node/package.json",
        r#"{"name":"@types/node","version":"20.0.0","types":"index.d.ts"}"#,
    );
    project.add_file(
        "node_modules/@types/node/index.d.ts",
        "/// <reference path=\"path.d.ts\" />\n/// <reference path=\"fs.d.ts\" />\n",
    );
    project.add_file(
        "node_modules/@types/node/path.d.ts",
        "declare module \"path\" {\n    namespace path {\n        interface PlatformPath {\n            join(...paths: string[]): string;\n            dirname(p: string): string;\n        }\n    }\n    const path: path.PlatformPath;\n    export = path;\n}\ndeclare module \"node:path\" {\n    import path = require(\"path\");\n    export = path;\n}\n",
    );
    project.add_file(
        "node_modules/@types/node/fs.d.ts",
        "declare module \"fs\" {\n    export function existsSync(p: string): boolean;\n}\n",
    );
    project.add_file(
        "node_modules/shim-less-pkg/package.json",
        r#"{"name":"shim-less-pkg","version":"1.0.0","main":"index.js"}"#,
    );
    project.add_file(
        "node_modules/shim-less-pkg/index.js",
        "module.exports = { shimmed() {} };\n",
    );
    project.add_file(
        "src/shims.d.ts",
        "declare module 'shim-less-pkg' {\n    export function shimmed(): void;\n}\n",
    );
    project.add_file(
        "src/use.ts",
        "import { join } from 'path';\nimport { existsSync } from 'fs';\nimport { join as njoin } from 'node:path';\nimport p from 'path';\nimport { shimmed } from 'shim-less-pkg';\nimport { nope } from 'path';\n\nexport function probe(root: string): boolean {\n  shimmed();\n  nope();\n  const direct = join(root, 'x.txt');\n  const aliased = njoin(root, 'y.txt');\n  return existsSync(direct) && aliased.length > p.dirname(root).length;\n}\n",
    );
    project
}

/// `(source, kind, target, target file)` for every edge leaving the consumer.
fn consumer_edges(db: &bearwisdom::Database) -> Vec<(String, String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT s.name, e.kind, t.name, f.path FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files f ON f.id = t.file_id
             WHERE s.name = 'probe' ORDER BY e.kind, t.name, f.path",
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

fn unresolved_names(db: &bearwisdom::Database) -> Vec<String> {
    let mut stmt = db
        .prepare("SELECT DISTINCT target_name FROM unresolved_refs ORDER BY target_name")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_declared_module_unit_is_the_module_its_importers_name() {
    let project = seed_project();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let declaring: Vec<String> = {
        let mut stmt = db
            .prepare("SELECT path FROM files WHERE path LIKE 'ext:ts:@types/node/%' ORDER BY path")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert!(
        declaring.contains(&"ext:ts:@types/node/path.d.ts".to_string())
            && declaring.contains(&"ext:ts:@types/node/fs.d.ts".to_string()),
        "the declaring sources are materialized: {declaring:?}"
    );

    let found = consumer_edges(&db);
    for expected in [
        // No export assignment is involved: the unit's own export binds.
        edge("probe", "calls", "existsSync", "ext:ts:@types/node/fs.d.ts"),
        // `export = path` publishes the assigned value's declared members,
        // reached both directly and through the re-assigning `node:path`.
        edge("probe", "calls", "join", "ext:ts:@types/node/path.d.ts"),
        // The default import binds the assigned value itself.
        // A project-owned, non-isolated ambient declaration provides too.
        edge("probe", "calls", "shimmed", "src/shims.d.ts"),
    ] {
        assert!(
            found.contains(&expected),
            "missing {expected:?} in {found:?}"
        );
    }

    assert!(
        !found.iter().any(|(_, _, target, _)| target == "nope"),
        "the member surface manufactures no member: {found:?}"
    );
    let unresolved = unresolved_names(&db);
    for linked in ["existsSync", "join", "njoin", "shimmed"] {
        assert!(
            !unresolved.contains(&linked.to_string()),
            "{linked} stayed unresolved: {unresolved:?}"
        );
    }
    assert!(
        unresolved.contains(&"nope".to_string()),
        "an absent export still fails: {unresolved:?}"
    );
}
