//! A dependency's exported value can be callable without being a function: its
//! declared type carries the call signature. Calling the imported binding must
//! yield that signature's return, and a property of the same callable type must
//! yield it too — so the member walk continues onto the returned declaration
//! instead of dead-ending on the type that merely describes the callable.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn index(project: &TestProject) -> bearwisdom::Database {
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();
    db
}

/// `(declaring file, source line)` for every CALL edge leaving the consumer
/// source that targets a symbol named `target`.
fn call_targets_named(db: &bearwisdom::Database, target: &str) -> Vec<(String, i64)> {
    let mut stmt = db
        .prepare(
            "SELECT f.path, e.source_line FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files sf ON sf.id = s.file_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files f ON f.id = t.file_id
             WHERE sf.path = 'src/app.ts' AND t.name = ?1 AND e.kind = 'calls'
             ORDER BY e.source_line, f.path",
        )
        .unwrap();
    stmt.query_map([target], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Undrained unresolved refs left by the consumer source, as
/// `(target name, cause kind)`.
fn unresolved(db: &bearwisdom::Database) -> Vec<(String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT u.target_name, COALESCE(u.cause_kind, '') FROM unresolved_refs u
             JOIN symbols s ON s.id = u.source_id
             JOIN files f ON f.id = s.file_id
             WHERE f.path = 'src/app.ts' AND u.drained = 0
             ORDER BY u.target_name",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// One package exporting a value whose declared type is callable: `CheckStatic`
/// carries an anonymous call signature returning `Check<T>`, and its `soft`
/// property is another value of that same callable type. The consumer imports
/// the value, so its chain roots on a source-bound import binding.
fn seed_callable_value(project: &TestProject) {
    project.add_file(
        "package.json",
        r#"{"name":"consumer","dependencies":{"callable-values":"*"}}"#,
    );
    project.add_file(
        "node_modules/callable-values/package.json",
        r#"{"name":"callable-values","types":"index.d.ts"}"#,
    );
    project.add_file(
        "node_modules/callable-values/index.d.ts",
        "export interface Check<T> {\n    toBe(value: T): void;\n}\n\nexport interface CheckStatic {\n    <T>(actual: T): Check<T>;\n    soft: CheckStatic;\n}\n\nexport declare const check: CheckStatic;\n",
    );
    project.add_file(
        "src/app.ts",
        "import { check } from 'callable-values';\n\nexport function verify(): void {\n    check(1).toBe(1);\n    check.soft(2).toBe(2);\n}\n",
    );
}

#[test]
fn calling_an_imported_callable_value_binds_its_call_signatures_return() {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    seed_callable_value(&project);
    let db = index(&project);

    let hits = call_targets_named(&db, "toBe");
    assert!(
        hits.iter()
            .all(|(path, _)| path == "ext:ts:callable-values/index.d.ts"),
        "`toBe` belongs to the package's `Check`: {hits:?}"
    );
    assert_eq!(
        hits.len(),
        2,
        "both `check(1)` and `check.soft(2)` yield `Check`, one edge per call site: {hits:?}"
    );
}

#[test]
fn a_callable_value_root_leaves_no_uncaptured_return() {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    seed_callable_value(&project);
    let db = index(&project);

    let left = unresolved(&db);
    assert!(
        !left.iter().any(|(_, cause)| cause == "uncaptured_return"),
        "the call signature supplies the root's yield, so nothing blames `check`: {left:?}"
    );
    assert!(
        !left.iter().any(|(target, _)| target == "toBe"),
        "every `toBe` hop binds: {left:?}"
    );
}
