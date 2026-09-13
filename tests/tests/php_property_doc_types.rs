//! A PHP property with no native type hint takes its type from its docblock's
//! `@var` tag, or from the `new` expression a method assigns into it. The tag's
//! spelling is preserved, so a fully-qualified tag binds to its own namespace
//! and not to a same-named class elsewhere in the index.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn case(class: &str, declaration: &str, body: &str) -> String {
    format!(
        "<?php\n\nnamespace Fx;\n\nclass {class}\n{{\n{declaration}\n\n    public function run()\n    {{\n{body}\n    }}\n}}\n"
    )
}

fn seed_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "composer.json",
        r#"{"name":"fixture/php-var-field-typing","autoload":{"psr-4":{"Fx\\":"src/"}}}"#,
    );
    project.add_file(
        "src/Factory.php",
        "<?php\n\nnamespace Fx;\n\nclass Factory\n{\n    public function fake()\n    {\n        return $this;\n    }\n}\n",
    );
    // A same-named class in a sibling namespace: a tag that collapsed to its
    // simple leaf would be free to bind here instead.
    project.add_file(
        "src/Other/Factory.php",
        "<?php\n\nnamespace Fx\\Other;\n\nclass Factory\n{\n    public function other()\n    {\n        return $this;\n    }\n}\n",
    );
    project.add_file(
        "src/ControlCase.php",
        &case(
            "ControlCase",
            "    protected Factory $factory;",
            "        $this->factory->fake();",
        ),
    );
    project.add_file(
        "src/DocVarCase.php",
        &case(
            "DocVarCase",
            "    /**\n     * The factory.\n     *\n     * @var \\Fx\\Factory\n     */\n    protected $factory;",
            "        $this->factory->fake();",
        ),
    );
    project.add_file(
        "src/ImportedDocCase.php",
        "<?php\n\nnamespace Fx;\n\nuse Fx\\Factory;\n\nclass ImportedDocCase\n{\n    /** @var Factory */\n    protected $factory;\n\n    public function run()\n    {\n        $this->factory->fake();\n    }\n}\n",
    );
    project.add_file(
        "src/AssignCase.php",
        &case(
            "AssignCase",
            "    protected $factory;\n\n    protected function setUp()\n    {\n        $this->factory = new Factory;\n    }",
            "        $this->factory->fake();",
        ),
    );
    // A tag whose class identity is not unambiguous is deliberately abstained:
    // the member call stays unresolved rather than being typed by a guess.
    project.add_file(
        "src/AbstainCase.php",
        &case(
            "AbstainCase",
            "    /** @var \\Fx\\Factory[] */\n    protected $many;",
            "        $this->many->fake();",
        ),
    );
    project
}

/// Source files of every `calls` edge that lands on `Fx\Factory::fake`.
fn callers_of_fake(db: &bearwisdom::Database) -> Vec<String> {
    let mut stmt = db
        .prepare(
            "SELECT sf.path FROM edges e
             JOIN symbols s  ON s.id  = e.source_id
             JOIN files   sf ON sf.id = s.file_id
             JOIN symbols t  ON t.id  = e.target_id
             JOIN files   tf ON tf.id = t.file_id
             WHERE e.kind = 'calls' AND t.name = 'fake' AND tf.path = 'src/Factory.php'
             ORDER BY sf.path",
        )
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Source files of every unresolved `fake` reference.
fn unresolved_fake(db: &bearwisdom::Database) -> Vec<String> {
    let mut stmt = db
        .prepare(
            "SELECT f.path FROM unresolved_refs u
             JOIN symbols s ON s.id = u.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE u.target_name = 'fake'
             ORDER BY f.path",
        )
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn edges_into_the_namesake(db: &bearwisdom::Database) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols t ON t.id = e.target_id
         JOIN files   f ON f.id = t.file_id
         WHERE f.path = 'src/Other/Factory.php'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn an_unhinted_property_is_typed_by_its_doc_tag_or_its_assignment() {
    let project = seed_project();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let callers = callers_of_fake(&db);
    for expected in [
        "src/AssignCase.php",
        "src/ControlCase.php",
        "src/DocVarCase.php",
        "src/ImportedDocCase.php",
    ] {
        assert!(
            callers.iter().any(|path| path == expected),
            "no calls edge from {expected} to Fx\\Factory::fake, got {callers:?}"
        );
    }

    assert_eq!(
        edges_into_the_namesake(&db),
        0,
        "a qualified @var tag must not bind to the same-named class in Fx\\Other"
    );

    assert_eq!(
        unresolved_fake(&db),
        vec!["src/AbstainCase.php".to_string()],
        "only the deliberately abstained tag leaves `fake` unresolved"
    );
}
