//! A local bound to a construction call carries the constructed declaration's
//! type: the constructor symbol's return slot yields the class it constructs,
//! so the member call on that local binds to the class's member.
//!
//! `BEARWISDOM_DART_SDK` is pointed at an EMPTY stub `lib/` so the probe chain
//! in `ecosystem/dart_sdk.rs` finds nothing and the fixture indexes the
//! project's own files only, whatever the host machine has installed.

use bearwisdom::full_index;
use bearwisdom::Database;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn seed_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "pubspec.yaml",
        "name: ctor_yield_fixture\nenvironment:\n  sdk: \">=3.0.0 <4.0.0\"\n",
    );
    project.add_file(
        "lib/analyzer.dart",
        "class StatefulAnalyzer {\n  StatefulAnalyzer(this.config);\n  final String config;\n  List<String> validateAll() => <String>[];\n}\n",
    );
    project.add_file(
        "lib/run.dart",
        "import 'analyzer.dart';\n\nvoid run() {\n  var analyzer = StatefulAnalyzer('local');\n  analyzer.validateAll();\n}\n",
    );
    project.add_file(
        "lib/chain.dart",
        "import 'analyzer.dart';\n\nvoid chained() {\n  var a = StatefulAnalyzer('x');\n  a.validateAll().length;\n}\n",
    );
    project
}

/// Index the fixture with the Dart SDK probe short-circuited.
fn index_fixture(project: &TestProject) -> Database {
    let sdk_stub = TempDir::new().unwrap();
    std::fs::create_dir_all(sdk_stub.path().join("lib")).unwrap();

    let prior_sdk = std::env::var_os("BEARWISDOM_DART_SDK");
    unsafe {
        std::env::set_var("BEARWISDOM_DART_SDK", sdk_stub.path());
    }

    let mut db = TestProject::in_memory_db();
    let result = full_index(&mut db, project.path(), None, None, None);

    unsafe {
        match prior_sdk {
            Some(v) => std::env::set_var("BEARWISDOM_DART_SDK", v),
            None => std::env::remove_var("BEARWISDOM_DART_SDK"),
        }
    }
    result.expect("index failed");
    db
}

/// `(source symbol, source file, target file)` for every resolved `calls` edge
/// whose target is named `callee`.
fn calls_to(db: &Database, callee: &str) -> Vec<(String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT s.name, sf.path, tf.path FROM edges e
             JOIN symbols s  ON s.id = e.source_id
             JOIN files   sf ON sf.id = s.file_id
             JOIN symbols t  ON t.id = e.target_id
             JOIN files   tf ON tf.id = t.file_id
             WHERE e.kind = 'calls' AND t.name = ?1
             ORDER BY sf.path, s.name",
        )
        .unwrap();
    stmt.query_map([callee], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Every undrained `untyped_binding` unresolved ref in the file at `path`.
fn untyped_bindings_in(db: &Database, path: &str) -> Vec<String> {
    let mut stmt = db
        .prepare(
            "SELECT u.target_name FROM unresolved_refs u
             JOIN symbols s ON s.id = u.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.path = ?1 AND u.drained = 0 AND u.cause_kind = 'untyped_binding'
             ORDER BY u.target_name",
        )
        .unwrap();
    stmt.query_map([path], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Both starved consumers in one index pass: the binding seed that types the
/// local, and the chain root that reads the same return slot. One test because
/// the SDK short-circuit mutates a process-wide environment variable, which two
/// concurrently running tests would race on.
#[test]
fn dart_local_bound_to_a_constructor_call_types_its_member_call() {
    let project = seed_project();
    let db = index_fixture(&project);
    let found = calls_to(&db, "validateAll");

    let direct = (
        "run".to_string(),
        "lib/run.dart".to_string(),
        "lib/analyzer.dart".to_string(),
    );
    assert!(
        found.contains(&direct),
        "the local's member call must bind to the constructed class's member; got {found:?}"
    );

    let chained = (
        "chained".to_string(),
        "lib/chain.dart".to_string(),
        "lib/analyzer.dart".to_string(),
    );
    assert!(
        found.contains(&chained),
        "a chained root also reads the constructor's return slot; got {found:?}"
    );

    let untyped = untyped_bindings_in(&db, "lib/run.dart");
    assert!(
        untyped.is_empty(),
        "the untyped-binding cause row must be gone, not shadowed: {untyped:?}"
    );
}
