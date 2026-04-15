//! Integration test for PHP Composer vendor/ externals pipeline.
//!
//! Seeds a temp directory that mimics a Composer project with:
//!   - composer.json declaring "fakevendor/helpers"
//!   - vendor/fakevendor/helpers/src/helpers.php  — global helper functions
//!   - vendor/fakevendor/helpers/src/Facades/Auth.php — namespaced facade
//!   - app/Controller.php — consumer calling route() and using Auth facade
//!
//! Asserts end-to-end:
//!   1. External files land with origin='external'
//!   2. External symbols indexed (route function, Auth class)
//!   3. Internal symbol search filters externals out
//!   4. At least one internal→external edge exists (Controller → Auth)

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;

fn build_php_project() -> TestProject {
    let project = TestProject {
        dir: tempfile::TempDir::new().unwrap(),
    };

    // composer.json declaring the fake dep
    project.add_file(
        "composer.json",
        r#"{"require":{"fakevendor/helpers":"^1.0"}}"#,
    );

    // vendor/fakevendor/helpers/composer.json — for version reading
    project.add_file(
        "vendor/fakevendor/helpers/composer.json",
        r#"{"name":"fakevendor/helpers","version":"1.0.0"}"#,
    );

    // Global helper functions wrapped in if(!function_exists()) guards —
    // mirrors the pattern used by laravel/framework helpers.php
    project.add_file(
        "vendor/fakevendor/helpers/src/helpers.php",
        r#"<?php

if (! function_exists('route')) {
    function route($name, $parameters = [], $absolute = true)
    {
        return app('url')->route($name, $parameters, $absolute);
    }
}

if (! function_exists('trans')) {
    function trans($key = null, $replace = [], $locale = null)
    {
        return app('translator')->get($key, $replace, $locale);
    }
}
"#,
    );

    // Namespaced facade class
    project.add_file(
        "vendor/fakevendor/helpers/src/Facades/Auth.php",
        r#"<?php

namespace Fakevendor\Helpers\Facades;

class Auth
{
    public static function user()
    {
        return static::$app['auth']->user();
    }

    public static function check()
    {
        return static::$app['auth']->check();
    }
}
"#,
    );

    // Consumer file — uses Auth facade via use statement and calls route()
    project.add_file(
        "app/Controller.php",
        r#"<?php

namespace App\Http\Controllers;

use Fakevendor\Helpers\Facades\Auth;

class UserController
{
    public function index()
    {
        if (Auth::check()) {
            $user = Auth::user();
            return route('dashboard');
        }
        return trans('messages.login');
    }
}
"#,
    );

    project
}

#[test]
fn external_php_vendor_is_indexed_and_resolved() {
    let project = build_php_project();

    let mut db = TestProject::in_memory_db();
    let stats = full_index(&mut db, project.path(), None, None, None).unwrap();

    // --- Assertion 1: file_count reflects internal files only ---
    // composer.json is not a PHP source file, so we expect only Controller.php
    assert!(
        stats.file_count >= 1 && stats.file_count <= 3,
        "expected 1-3 internal files (Controller.php + possibly composer.json/vendor composer.json), got {}",
        stats.file_count
    );

    // --- Assertion 2: external PHP files landed in DB ---
    let external_files: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM files WHERE origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        external_files >= 1,
        "expected at least one external PHP file (helpers.php or Auth.php), got {external_files}"
    );

    // --- Assertion 3: external symbols indexed (route + trans + Auth) ---
    let external_symbols: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        external_symbols >= 2,
        "expected at least route + Auth external symbols, got {external_symbols}"
    );

    // --- Assertion 4: internal search hides external symbols ---
    let search_hits =
        bearwisdom::query::search::search_symbols(&db, "route", 10, &Default::default()).unwrap();
    assert!(
        search_hits
            .iter()
            .all(|s| !s.file_path.starts_with("ext:")),
        "search_symbols leaked an external symbol: {:?}",
        search_hits
            .iter()
            .map(|s| &s.file_path)
            .collect::<Vec<_>>()
    );

    // --- Assertion 5: internal→external edge exists (Controller uses Auth) ---
    let edges_to_external: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN symbols s ON s.id = e.target_id
             WHERE s.origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        edges_to_external >= 1,
        "expected at least one internal→external edge (Controller → Auth), got {edges_to_external}"
    );
}
