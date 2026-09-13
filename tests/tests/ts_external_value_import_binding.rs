//! A dependency's exported value is annotated with a type it imports from a
//! SIBLING package. That annotation is retained as a module signature, so the
//! value's type is whatever the module graph resolves the import to — there is
//! no second, spelling-based writer behind it. When the graph declines, the
//! value is typeless and every call through it dies as `uncaptured_return`.
//!
//! The second layout is the shape the corpus hits: the package declaring the
//! type carries one construct the binder cannot read, so its module surface is
//! only partly read. The names it DID declare must still answer.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn index(project: &TestProject) -> bearwisdom::Database {
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();
    db
}

/// The declaring file of every CALL edge leaving the consumer source that
/// targets a symbol named `target`.
fn call_target_files(db: &bearwisdom::Database, target: &str) -> Vec<String> {
    let mut stmt = db
        .prepare(
            "SELECT f.path FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files sf ON sf.id = s.file_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files f ON f.id = t.file_id
             WHERE sf.path = 'src/app.ts' AND t.name = ?1 AND e.kind = 'calls'
             ORDER BY f.path",
        )
        .unwrap();
    stmt.query_map([target], |r| r.get(0))
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

/// `matchers` owns the call signature and the assertion type; `runner`
/// re-exports a value whose only type evidence is `matchers`' interface,
/// reached through an import of a second package.
fn seed_two_packages(project: &TestProject, matchers_tail: &str) {
    project.add_file(
        "package.json",
        r#"{"name":"consumer","dependencies":{"runner":"*"}}"#,
    );
    project.add_file(
        "node_modules/matchers/package.json",
        r#"{"name":"matchers","types":"index.d.ts"}"#,
    );
    project.add_file(
        "node_modules/matchers/index.d.ts",
        &format!(
            "interface Check<T> {{\n    toBe(value: T): void;\n}}\n\n\
             interface CheckStatic {{\n    <T>(actual: T): Check<T>;\n}}\n\n\
             export {{ Check, CheckStatic }};\n{matchers_tail}"
        ),
    );
    project.add_file(
        "node_modules/runner/package.json",
        r#"{"name":"runner","types":"index.d.ts","dependencies":{"matchers":"*"}}"#,
    );
    project.add_file(
        "node_modules/runner/index.d.ts",
        "import { CheckStatic } from 'matchers';\n\n\
         declare const globalCheck: CheckStatic;\n\n\
         export { globalCheck as check };\n",
    );
    project.add_file(
        "src/app.ts",
        "import { check } from 'runner';\n\n\
         export function verify(): void {\n    check(1).toBe(1);\n}\n",
    );
}

fn project_with(matchers_tail: &str) -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    seed_two_packages(&project, matchers_tail);
    project
}

fn assert_call_binds_through_both_packages(db: &bearwisdom::Database, layout: &str) {
    assert_eq!(
        call_target_files(db, "toBe"),
        ["ext:ts:matchers/index.d.ts"],
        "{layout}: `check(1)` yields `matchers`' `Check`, so `toBe` lands there"
    );
    let left = unresolved(db);
    assert!(
        !left.iter().any(|(_, cause)| cause == "uncaptured_return"),
        "{layout}: the imported value carries its sibling package's type: {left:?}"
    );
}

#[test]
fn an_imported_values_cross_package_annotation_types_its_call() {
    let db = index(&project_with(""));
    assert_call_binds_through_both_packages(&db, "whole");
}

/// A grammar gap inside a declaration body of the matchers package (a mapped
/// type modifier written `]? :`) must not make the package's exports
/// unreadable: the imported value still types through it.
#[test]
fn a_package_with_an_error_inside_a_declaration_body_still_types_the_imported_value() {
    let db = index(&project_with(
        "
type Partialish<T> = { [K in keyof T]? : T[K] };
export { Partialish };
",
    ));
    assert_call_binds_through_both_packages(&db, "an error confined to a declaration body");
}
