//! A Python instance attribute assigned in a method body is a member of the
//! class, and it carries the type its initializer states: a constructor call
//! names the class directly, a bare name takes the annotation of the parameter
//! it names. A member whose initializer states no type stays untyped — the
//! walk through it must not guess.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

/// One module declaring two collaborators and a service that holds both as
/// instance attributes. Single-module so the assertions isolate attribute
/// typing from import linking.
fn seed_service() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "svc.py",
        r#"class Repo:
    def find(self):
        return 1


class Cache:
    def get(self):
        return 2


class Svc:
    def __init__(self, repo: Repo, thing):
        self.repo = repo
        self.cache = Cache()
        self.thing = thing

    def run(self):
        self.repo.find()
        self.cache.get()

    def guess(self):
        self.thing.unknowable()
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
fn an_instance_attribute_carries_the_type_its_initializer_states() {
    let project = seed_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let declared = symbols(&db);
    for expected in [
        ("Svc.repo".to_string(), "property".to_string()),
        ("Svc.cache".to_string(), "property".to_string()),
        ("Svc.thing".to_string(), "property".to_string()),
    ] {
        assert!(
            declared.contains(&expected),
            "missing {expected:?} in {declared:?}"
        );
    }
    // The assignment declares the member once, on the class — not once per
    // assigning method, and never under the method itself.
    assert_eq!(
        declared
            .iter()
            .filter(|(qname, _)| qname == "Svc.repo")
            .count(),
        1,
        "{declared:?}"
    );

    let found = edges(&db);
    for expected in [
        // The annotated constructor parameter types the member it initializes.
        edge("Svc.repo", "type_ref", "Repo"),
        edge("Svc.run", "calls", "Repo.find"),
        // A constructor-call initializer types it directly.
        edge("Svc.cache", "type_ref", "Cache"),
        edge("Svc.run", "calls", "Cache.get"),
    ] {
        assert!(found.contains(&expected), "missing {expected:?} in {found:?}");
    }

    // The unannotated parameter states no type, so the member it initializes
    // states none either: the call through it stays unresolved rather than
    // binding to a same-named member of some other class.
    let unresolved = unresolved_calls(&db);
    assert!(
        unresolved.contains(&"unknowable".to_string()),
        "{unresolved:?}"
    );
    assert!(
        !unresolved.contains(&"find".to_string()) && !unresolved.contains(&"get".to_string()),
        "{unresolved:?}"
    );
}
