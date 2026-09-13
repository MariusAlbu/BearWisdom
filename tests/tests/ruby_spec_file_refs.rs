//! An RSpec suite declares nothing: `describe`/`it` are method calls taking
//! blocks, so the file has no class and no method of its own. Its references
//! to project code still have to reach the graph, which means the file needs
//! one symbol to own them — a file-scope owner the indexer materializes when
//! extraction produced references but no declaration.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

const SPEC_FILE: &str = "spec/widget_spec.rb";
const LIB_FILE: &str = "lib/widget.rb";

fn seed_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(LIB_FILE, "class Widget\n  def spin\n  end\nend\n");
    project.add_file(
        SPEC_FILE,
        "require 'widget'\n\ndescribe Widget do\n  it 'spins' do\n    w = Widget.new\n    w.spin\n  end\nend\n",
    );
    project
}

/// Every symbol declared in `path`, as `(name, kind, qualified_name)`.
fn symbols_in(db: &bearwisdom::Database, path: &str) -> Vec<(String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT s.name, s.kind, s.qualified_name FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path = ?1
             ORDER BY s.name, s.kind",
        )
        .unwrap();
    stmt.query_map([path], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Every edge leaving `path`, as `(kind, target name, target file)`.
fn edges_from(db: &bearwisdom::Database, path: &str) -> Vec<(String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT e.kind, t.name, tf.path FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files sf ON sf.id = s.file_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files tf ON tf.id = t.file_id
             WHERE sf.path = ?1
             ORDER BY e.kind, t.name",
        )
        .unwrap();
    stmt.query_map([path], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_declaration_less_spec_file_gets_exactly_one_owner_symbol() {
    let project = seed_project();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let symbols = symbols_in(&db, SPEC_FILE);
    let owners: Vec<_> = symbols
        .iter()
        .filter(|(_, kind, _)| kind == "module")
        .collect();
    assert_eq!(
        owners,
        vec![&(
            "widget_spec".to_string(),
            "module".to_string(),
            "spec/widget_spec".to_string(),
        )],
        "the suite gets exactly one file-scope owner: {symbols:?}",
    );
    // The suite's own bindings (`w = Widget.new`) synthesize under the owner,
    // the way a method's bindings do under the method.
    assert!(
        symbols
            .iter()
            .all(|(_, kind, _)| kind == "module" || kind == "variable"),
        "nothing but the owner and its bindings is declared: {symbols:?}",
    );

    // The declaring file is untouched: it declares its own symbols, so no
    // owner is added on top of them.
    let declared: Vec<String> = symbols_in(&db, LIB_FILE)
        .into_iter()
        .map(|(name, _, _)| name)
        .collect();
    assert!(
        declared.contains(&"Widget".to_string()) && declared.contains(&"spin".to_string()),
        "the class and its method are declared: {declared:?}",
    );
    assert!(
        !declared.contains(&"widget".to_string()),
        "a file that declares symbols gets no file-scope owner: {declared:?}",
    );
}

#[test]
fn the_owner_carries_the_suite_refs_into_the_graph() {
    let project = seed_project();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let edges = edges_from(&db, SPEC_FILE);
    assert!(
        edges.iter().any(|(kind, target, file)| target == "Widget"
            && file == LIB_FILE
            && (kind == "type_ref" || kind == "calls" || kind == "instantiates")),
        "the class the suite exercises is reached: {edges:?}",
    );
    assert!(
        edges
            .iter()
            .any(|(_, target, file)| target == "spin" && file == LIB_FILE),
        "the method the suite calls is reached: {edges:?}",
    );
}
