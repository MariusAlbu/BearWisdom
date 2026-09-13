//! A bare call inside a namespaced file falls back to the ROOT namespace for
//! functions and constants, but never for classes: `data_get` / `url` bind the
//! project's own root declarations, `vendor_helper` binds the vendor declaration, and
//! a bare class mention stays unresolved because the root-fallback kind set
//! excludes types.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;

fn build_project() -> TestProject {
    let project = TestProject {
        dir: tempfile::TempDir::new().unwrap(),
    };

    project.add_file(
        "composer.json",
        r#"{"name":"acme/app","require":{"acme/stubs":"^1.0"},"autoload":{"psr-4":{"App\\":"src/"}}}"#,
    );

    project.add_file(
        "vendor/acme/stubs/composer.json",
        r#"{"name":"acme/stubs","version":"1.0.0"}"#,
    );

    // A supplied root declaration: no namespace, so its qualified name is bare.
    project.add_file(
        "vendor/acme/stubs/src/standard.php",
        r#"<?php

function vendor_helper($value): bool {}
"#,
    );

    // The project's own root namespace: two helper functions and one class.
    project.add_file(
        "src/helpers.php",
        r#"<?php

use App\Support\Arr;

class RootOnly
{
}

if (! function_exists('data_get')) {
    function data_get($target, $key)
    {
        return Arr::get($target, $key);
    }
}

if (! function_exists('url')) {
    function url($path = null)
    {
        return $path;
    }
}
"#,
    );

    // A namespaced consumer. Bare calls fall back to the root; the bare class
    // mention does not.
    project.add_file(
        "src/Support/Arr.php",
        r#"<?php

namespace App\Support;

class Arr
{
    public static function get($target, $key)
    {
        return $target[$key];
    }

    public static function pluck($array, $value)
    {
        $internal = data_get($array, $value);
        $link = url($value);
        $empty = vendor_helper($internal);

        return [$internal, $link, $empty, self::get($array, $value)];
    }

    public function describe(RootOnly $root)
    {
        return $root;
    }
}
"#,
    );

    project
}

fn root_namespace_edges(db: &bearwisdom::Database) -> Vec<(String, String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT s.name, t.name, f.path FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files f ON f.id = t.file_id
             WHERE e.strategy = 'root_namespace'
             ORDER BY s.name, t.name",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn bare_functions_fall_back_to_the_root_namespace() {
    let project = build_project();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let found = root_namespace_edges(&db);

    for (source, target) in [("pluck", "data_get"), ("pluck", "url")] {
        assert!(
            found
                .iter()
                .any(|(s, t, f)| s == source && t == target && f == "src/helpers.php"),
            "missing {source} -> {target} in src/helpers.php: {found:?}"
        );
    }
    assert!(
        found
            .iter()
            .any(|(s, t, f)| s == "pluck" && t == "vendor_helper" && f.contains("standard.php")),
        "missing pluck -> vendor_helper on the vendor declaration: {found:?}"
    );
}

#[test]
fn the_root_fallback_binds_no_kind_outside_the_profile_set() {
    let project = build_project();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let off_kind: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e JOIN symbols t ON t.id = e.target_id
             WHERE e.strategy = 'root_namespace' AND t.kind NOT IN ('function', 'field')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(off_kind, 0, "the kind set must fence out every other kind");

    let root_class: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e JOIN symbols t ON t.id = e.target_id
             WHERE e.strategy = 'root_namespace' AND t.name = 'RootOnly'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        root_class, 0,
        "a bare class mention must not reach the root declaration"
    );
}

#[test]
fn an_enclosing_member_still_binds_through_the_member_walk() {
    let project = build_project();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let strategies: Vec<Option<String>> = db
        .prepare(
            "SELECT e.strategy FROM edges e
             JOIN symbols t ON t.id = e.target_id
             WHERE e.kind = 'calls' AND t.qualified_name = 'App.Support.Arr.get'",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    assert!(
        !strategies.is_empty(),
        "Arr::get must still bind through the member walk"
    );
    assert!(
        strategies
            .iter()
            .all(|s| s.as_deref() != Some("root_namespace")),
        "a class member must not be reached by the root fallback: {strategies:?}"
    );
}
