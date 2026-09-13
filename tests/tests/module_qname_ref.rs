//! A ref whose extractor-set `module` IS the indexed qualified name of the
//! declaration it names binds to that declaration, with no import entry in the
//! file to lean on. The member call under the same module keeps binding to the
//! member, so the module-as-container reading survives alongside it.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn seed_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    // No `alias` directive: the file import table stays empty, so only the
    // ref's own module spelling can name the declaration.
    project.add_file(
        "lib/plausible/repo.ex",
        "defmodule Plausible.Repo do\n  def all(q), do: q\nend\n",
    );
    project.add_file(
        "lib/plausible/audit.ex",
        "defmodule Plausible.Audit do\n  def list(q) do\n    Plausible.Repo.all(q)\n  end\nend\n",
    );
    project.add_file(
        "src/Text/Pandoc/Options.hs",
        "module Text.Pandoc.Options where\ndata ReaderOptions = ReaderOptions\n",
    );
    project.add_file(
        "src/Text/Pandoc/Reader.hs",
        "module Text.Pandoc.Reader where\nimport Text.Pandoc.Options\n",
    );
    project
}

/// `(source file, edge kind, target qualified name)` for every resolved edge.
fn edges_by_target_qname(db: &bearwisdom::Database) -> Vec<(String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT sf.path, e.kind, t.qualified_name FROM edges e
             JOIN symbols s  ON s.id = e.source_id
             JOIN files   sf ON sf.id = s.file_id
             JOIN symbols t  ON t.id = e.target_id
             ORDER BY sf.path, e.kind, t.qualified_name",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn edge(file: &str, kind: &str, target_qname: &str) -> (String, String, String) {
    (file.into(), kind.into(), target_qname.into())
}

#[test]
fn a_module_spelling_that_is_a_declaration_qname_binds_to_that_declaration() {
    let project = seed_project();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let found = edges_by_target_qname(&db);
    for expected in [
        // The module spelling names the declaration itself.
        edge("lib/plausible/audit.ex", "type_ref", "Plausible.Repo"),
        // The member under the same module still binds to the member.
        edge("lib/plausible/audit.ex", "calls", "Plausible.Repo.all"),
        // A module header symbol carries no kind row for `imports`, so the
        // namespace declaration is an admissible target.
        edge(
            "src/Text/Pandoc/Reader.hs",
            "imports",
            "Text.Pandoc.Options",
        ),
    ] {
        assert!(
            found.contains(&expected),
            "missing {expected:?} in {found:?}"
        );
    }

    let self_loops: Vec<(String, String, String)> = {
        let mut stmt = db
            .prepare(
                "SELECT s.qualified_name, e.kind, e.strategy FROM edges e
                 JOIN symbols s ON s.id = e.source_id
                 WHERE e.source_id = e.target_id",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert!(
        self_loops.is_empty(),
        "a declaration header never binds to itself: {self_loops:?}"
    );

    let leftover: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs WHERE target_name IN ('Repo', 'Options')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leftover, 0, "module-qname refs left unresolved");
}
