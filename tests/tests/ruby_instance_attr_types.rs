//! A Ruby instance variable assigned in a method body is a member of the class
//! that declares the method, and the call through it walks from the implicit
//! receiver. Its type is whatever the initializer states: `Cache.new` names the
//! class, an unannotated parameter names nothing — Ruby states no parameter
//! types, so the member it initializes stays untyped and the walk through it
//! must decline rather than guess.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

/// One file declaring a collaborator and a service holding it in an instance
/// variable. Single-file so the assertions isolate instance-variable typing
/// from constant resolution across files.
fn seed_service() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "svc.rb",
        r#"class Cache
  def get
    2
  end
end

class Svc
  def initialize(repo)
    @cache = Cache.new
    @repo = repo
  end

  def run
    @cache.get
  end

  def guess
    @repo.unknowable
  end
end
"#,
    );
    project
}

/// Every edge as `(source qualified name, kind, target qualified name)`.
fn edges(db: &bearwisdom::Database) -> Vec<(String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT s.qualified_name, e.kind, t.qualified_name FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN symbols t ON t.id = e.target_id
             ORDER BY s.qualified_name, e.kind, t.qualified_name",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn edge(source: &str, kind: &str, target: &str) -> (String, String, String) {
    (source.into(), kind.into(), target.into())
}

/// Every symbol as `(qualified name, kind)`.
fn symbols(db: &bearwisdom::Database) -> Vec<(String, String)> {
    let mut stmt = db
        .prepare("SELECT s.qualified_name, s.kind FROM symbols s ORDER BY s.qualified_name")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The names still unresolved as calls.
fn unresolved_calls(db: &bearwisdom::Database) -> Vec<String> {
    let mut stmt = db
        .prepare(
            "SELECT u.target_name FROM unresolved_refs u
             WHERE u.kind = 'calls' AND u.drained = 0
             ORDER BY u.target_name",
        )
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn an_instance_variable_is_a_typed_member_of_its_class() {
    let project = seed_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let declared = symbols(&db);
    for expected in [
        ("Svc.@cache".to_string(), "property".to_string()),
        ("Svc.@repo".to_string(), "property".to_string()),
    ] {
        assert!(
            declared.contains(&expected),
            "missing {expected:?} in {declared:?}"
        );
    }

    let found = edges(&db);
    // `Cache.new` names the member's class, and the call through the member
    // walks from the implicit receiver to `Cache#get`.
    assert!(
        found.contains(&edge("Svc.run", "calls", "Cache.get")),
        "missing Cache.get in {found:?}"
    );

    // Ruby annotates no parameter, so `@repo` has no type to walk: the call
    // through it stays unresolved rather than binding to a same-named method
    // of some other class.
    let unresolved = unresolved_calls(&db);
    assert!(
        unresolved.contains(&"unknowable".to_string()),
        "{unresolved:?}"
    );
    assert!(!unresolved.contains(&"get".to_string()), "{unresolved:?}");
}
