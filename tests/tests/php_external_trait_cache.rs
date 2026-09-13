//! A vendor trait is indexed as a trait and reached through an aliased parent:
//! `class Clock extends BaseClock` where `BaseClock` is `use`d under an alias,
//! and the vendor class pulls its members from `use Creator;`. The external
//! parse cache serves the extraction, so the cached kind must be the extractor's
//! current one.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn seed_aliased_trait_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "composer.json",
        r#"{"name":"acme/fixture","require":{"acme/clock":"^1.0"},"autoload":{"psr-4":{"App\\":"src/"}}}"#,
    );
    project.add_file(
        "vendor/acme/clock/composer.json",
        r#"{"name":"acme/clock","version":"1.0.0","autoload":{"psr-4":{"Acme\\Clock\\":"src/Clock/"}}}"#,
    );
    project.add_file(
        "vendor/acme/clock/src/Clock/Traits/Creator.php",
        "<?php\n\nnamespace Acme\\Clock\\Traits;\n\ntrait Creator\n{\n    public static function now(): static\n    {\n        return new static();\n    }\n}\n",
    );
    project.add_file(
        "vendor/acme/clock/src/Clock/Clock.php",
        "<?php\n\nnamespace Acme\\Clock;\n\nuse Acme\\Clock\\Traits\\Creator;\n\n/**\n * @method $this addSeconds(int $value = 1)\n */\nclass Clock\n{\n    use Creator;\n\n    public function isMutable(): bool\n    {\n        return true;\n    }\n}\n",
    );
    project.add_file(
        "src/Support/Clock.php",
        "<?php\n\nnamespace App\\Support;\n\nuse Acme\\Clock\\Clock as BaseClock;\n\nclass Clock extends BaseClock\n{\n}\n",
    );
    project.add_file(
        "src/Usage.php",
        "<?php\n\nnamespace App;\n\nuse App\\Support\\Clock;\n\nclass Usage\n{\n    public function go(): void\n    {\n        Clock::now()->addSeconds(5);\n    }\n}\n",
    );
    project
}

fn count(db: &bearwisdom::Database, sql: &str) -> i64 {
    db.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn an_external_trait_is_reached_through_an_aliased_parent() {
    let cache_dir = TempDir::new().unwrap();
    std::env::set_var("BEARWISDOM_CACHE_DIR", cache_dir.path());

    let project = seed_aliased_trait_project();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let external_traits = count(
        &db,
        "SELECT COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.name = 'Creator' AND s.kind = 'trait' AND f.origin = 'external'",
    );
    assert_eq!(external_traits, 1, "the vendor trait is indexed as a trait");

    let now_unresolved = count(
        &db,
        "SELECT COUNT(*) FROM unresolved_refs WHERE target_name = 'now' AND drained = 0",
    );
    assert_eq!(
        now_unresolved, 0,
        "Clock::now() climbs the aliased parent into the trait"
    );

    let now_edges = count(
        &db,
        "SELECT COUNT(*) FROM edges e JOIN symbols t ON t.id = e.target_id
         WHERE t.qualified_name = 'Acme.Clock.Traits.Creator.now'",
    );
    assert_eq!(now_edges, 1, "the call lands on the trait member");

    let doc_method = count(
        &db,
        "SELECT COUNT(*) FROM unresolved_refs WHERE target_name = 'addSeconds' AND drained = 0",
    );
    assert_eq!(
        doc_method, 1,
        "a PHPDoc @method virtual member is not a declaration"
    );
}
