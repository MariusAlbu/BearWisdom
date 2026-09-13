//! A member declared several times on ONE owner is that member under several
//! signatures, not a field of competing declarations. The hop binds it and the
//! chain carries on through its yield.
//!
//! `buffer.append("x").render()` in `app/Zqapp.java` steps to `app.Zqbuffer`'s
//! two-row `append`, takes its `app.Zqreport` yield, and binds `render`.
//! Treating the two rows as an ambiguous level kills the chain at the first
//! hop and leaves both `append` and `render` unresolved.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn seed_overloaded_member() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    // Both rows yield the same type, so the chain's next hop does not depend
    // on WHICH row represents the group — only on the group binding at all.
    project.add_file(
        "src/main/java/app/Zqbuffer.java",
        "package app;\npublic class Zqbuffer {\n    public Zqreport append(String text) { return null; }\n    public Zqreport append(int count) { return null; }\n}\n",
    );
    project.add_file(
        "src/main/java/app/Zqreport.java",
        "package app;\npublic class Zqreport {\n    public String render() { return \"\"; }\n}\n",
    );
    project.add_file(
        "src/main/java/app/Zqapp.java",
        "package app;\npublic class Zqapp {\n    public void run(Zqbuffer buffer) {\n        buffer.append(\"x\").render();\n    }\n}\n",
    );
    project
}

fn edges_to(db: &bearwisdom::Database, qname: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols t ON t.id = e.target_id
         WHERE t.qualified_name = ?1",
        [qname],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn an_owners_overload_set_binds_and_carries_the_chain_through_its_yield() {
    let project = seed_overloaded_member();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    assert!(
        edges_to(&db, "app.Zqbuffer.append") >= 1,
        "the two `append` rows are one member of Zqbuffer — the hop must bind it"
    );
    assert!(
        edges_to(&db, "app.Zqreport.render") >= 1,
        "the bound overload's yield must carry the chain to Zqreport.render"
    );

    let stalled: Vec<(String, Option<String>)> = {
        let mut stmt = db
            .prepare(
                "SELECT u.target_name, u.cause_kind FROM unresolved_refs u
                 JOIN symbols s ON s.id = u.source_id
                 JOIN files f ON f.id = s.file_id
                 WHERE u.drained = 0
                   AND f.path LIKE '%Zqapp.java'
                   AND u.target_name IN ('append', 'render')
                 ORDER BY u.target_name",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert!(
        stalled.is_empty(),
        "neither hop may survive as an unresolved ref: {stalled:?}"
    );
}
