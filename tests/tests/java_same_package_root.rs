//! A bare chain root whose simple name several packages declare binds to the
//! declaration in the caller's OWN package. No import exists — the file's
//! declared namespace is the evidence, and it must outrank a same-named type
//! that only shares a path prefix.
//!
//! `repo.findOne().getEmail()` in `app/App.java` types `repo` to `app.Zqrepo`,
//! walks `findOne` to `app.Zqentity`, and binds `getEmail`. Picking
//! `other.Zqrepo` instead kills the chain at the first hop — that homonym has
//! no `findOne`.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn seed_two_packages_one_name() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "src/main/java/app/Zqrepo.java",
        "package app;\npublic class Zqrepo {\n    public Zqentity findOne() { return null; }\n}\n",
    );
    project.add_file(
        "src/main/java/app/Zqentity.java",
        "package app;\npublic class Zqentity {\n    public String getEmail() { return \"\"; }\n}\n",
    );
    project.add_file(
        "src/main/java/other/Zqrepo.java",
        "package other;\npublic class Zqrepo {\n    public void unrelated() { }\n}\n",
    );
    project.add_file(
        "src/main/java/app/App.java",
        "package app;\npublic class App {\n    public void run(Zqrepo repo) {\n        repo.findOne().getEmail();\n    }\n}\n",
    );
    project
}

#[test]
fn a_bare_root_binds_the_homonym_declared_in_the_callers_own_package() {
    let project = seed_two_packages_one_name();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let resolved: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN symbols t ON t.id = e.target_id
             WHERE t.qualified_name = 'app.Zqentity.getEmail'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        resolved >= 1,
        "the same-package root must carry the chain through to app.Zqentity.getEmail"
    );

    let stolen: Vec<String> = {
        let mut stmt = db
            .prepare(
                "SELECT t.qualified_name FROM edges e
                 JOIN symbols t ON t.id = e.target_id
                 WHERE t.qualified_name LIKE 'other.%'
                 ORDER BY t.qualified_name",
            )
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert!(
        stolen.is_empty(),
        "nothing in the fixture names the `other` package: {stolen:?}"
    );
}
