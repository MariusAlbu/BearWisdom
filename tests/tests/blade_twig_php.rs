//! Integration tests for E2 — PHP ecosystem expansion.
//!
//! Verifies that:
//!   * `.php` files with inline `<script>` blocks now produce JS symbols
//!     via the new PHP embedded_regions hook.
//!   * `.blade.php` files surface directive symbols (sections, includes,
//!     extends) and route their `{{ }}` / `@php` content through the
//!     PHP sub-extractor.
//!   * `.twig` files surface block / macro symbols and template-extends
//!     edges.

use std::fs;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

#[test]
fn php_with_inline_script_extracts_js_symbols() {
    // E2 acceptance #1.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("page.php"),
        r#"<?php $name = 'Alice'; ?>
<html>
  <body>
    <script>
function onReady() {
  document.title = 'Hi';
}
function onClick() {}
    </script>
  </body>
</html>
"#,
    ).unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let js_names: Vec<String> = db.prepare(
        "SELECT s.name FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE f.path LIKE '%page.php'
           AND s.origin_language = 'javascript'
         ORDER BY s.name"
    ).unwrap()
     .query_map([], |r| r.get::<_, String>(0)).unwrap()
     .flatten().collect();

    assert!(js_names.contains(&"onReady".to_string()),
        "expected onReady JS symbol, got {js_names:?}");
    assert!(js_names.contains(&"onClick".to_string()),
        "expected onClick JS symbol, got {js_names:?}");
}

#[test]
fn blade_template_extracts_section_and_include_symbols() {
    // E2 acceptance #2.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("resources/views")).unwrap();
    fs::write(
        root.join("resources/views/welcome.blade.php"),
        r#"@extends('layouts.app')

@section('title', 'Welcome')

@section('content')
<h1>Hi {{ $user->name }}</h1>
@include('partials.footer')
@endsection
"#,
    ).unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // Host file symbol named after the template path.
    let host_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE f.path LIKE '%welcome.blade.php'
           AND s.qualified_name = 'welcome'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(host_count, 1, "expected exactly one 'welcome' host symbol");

    // Section symbols.
    let section_names: Vec<String> = db.prepare(
        "SELECT s.name FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE f.path LIKE '%welcome.blade.php'
           AND s.kind = 'method'
         ORDER BY s.name"
    ).unwrap()
     .query_map([], |r| r.get::<_, String>(0)).unwrap()
     .flatten().collect();
    assert!(section_names.contains(&"content".to_string()),
        "missing 'content' section, got {section_names:?}");
    assert!(section_names.contains(&"title".to_string()),
        "missing 'title' section, got {section_names:?}");

    // Imports refs from @extends and @include — surface as either
    // unresolved or external since no template resolution exists yet.
    let imports_targets: Vec<String> = db.prepare(
        "SELECT target_name FROM unresolved_refs ur
         JOIN symbols s ON s.id = ur.source_id
         JOIN files   f ON f.id = s.file_id
         WHERE f.path LIKE '%welcome.blade.php'
         UNION
         SELECT target_name FROM external_refs er
         JOIN symbols s ON s.id = er.source_id
         JOIN files   f ON f.id = s.file_id
         WHERE f.path LIKE '%welcome.blade.php'"
    ).unwrap()
     .query_map([], |r| r.get::<_, String>(0)).unwrap()
     .flatten().collect();
    assert!(imports_targets.contains(&"layouts.app".to_string()),
        "missing @extends ref, got {imports_targets:?}");
    assert!(imports_targets.contains(&"partials.footer".to_string()),
        "missing @include ref, got {imports_targets:?}");
}

#[test]
fn blade_routes_inline_php_through_php_extractor() {
    // E2: `{{ }}` / `@php @endphp` blocks must dispatch to the PHP
    // grammar so type / function refs in the expressions surface.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("resources/views")).unwrap();
    fs::write(
        root.join("resources/views/dashboard.blade.php"),
        r#"@php
function helper() { return 42; }
@endphp
<p>{{ helper() }}</p>
"#,
    ).unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // The @php block defines `helper`. Symbol must surface with
    // origin_language='php' (sub-extracted) on the Blade host file.
    let php_syms: Vec<String> = db.prepare(
        "SELECT s.name FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE f.path LIKE '%dashboard.blade.php'
           AND s.origin_language = 'php'"
    ).unwrap()
     .query_map([], |r| r.get::<_, String>(0)).unwrap()
     .flatten().collect();
    assert!(php_syms.contains(&"helper".to_string()),
        "expected php-origin 'helper' from @php block, got {php_syms:?}");
}

#[test]
fn twig_template_extracts_blocks_and_extends() {
    // E2 acceptance #3.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(
        root.join("templates/base.html.twig"),
        r#"<!doctype html>
<html>
<body>
{% block content %}{% endblock %}
{% block footer %}{% endblock %}
</body>
</html>
"#,
    ).unwrap();
    fs::write(
        root.join("templates/page.html.twig"),
        r#"{% extends "base.html.twig" %}

{% block content %}
<h1>Hello</h1>
{% include "partials/header.html.twig" %}
{% endblock %}
"#,
    ).unwrap();
    fs::create_dir_all(root.join("templates/partials")).unwrap();
    fs::write(
        root.join("templates/partials/header.html.twig"),
        "<header>x</header>",
    ).unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let blocks: Vec<String> = db.prepare(
        "SELECT s.qualified_name FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.kind = 'method'
         ORDER BY s.qualified_name"
    ).unwrap()
     .query_map([], |r| r.get::<_, String>(0)).unwrap()
     .flatten().collect();
    assert!(blocks.contains(&"base.content".to_string()),
        "missing base.content block, got {blocks:?}");
    assert!(blocks.contains(&"base.footer".to_string()),
        "missing base.footer block, got {blocks:?}");
    assert!(blocks.contains(&"page.content".to_string()),
        "missing page.content block, got {blocks:?}");

    // E2 acceptance #4: extends + include refs.
    let refs: Vec<String> = db.prepare(
        "SELECT target_name FROM unresolved_refs ur
         JOIN symbols s ON s.id = ur.source_id
         JOIN files   f ON f.id = s.file_id
         WHERE f.path LIKE '%page.html.twig'
         UNION
         SELECT er.target_name FROM external_refs er
         JOIN symbols s ON s.id = er.source_id
         JOIN files   f ON f.id = s.file_id
         WHERE f.path LIKE '%page.html.twig'
         UNION
         SELECT s2.name FROM edges e
         JOIN symbols src ON src.id = e.source_id
         JOIN files f ON f.id = src.file_id
         JOIN symbols s2 ON s2.id = e.target_id
         WHERE f.path LIKE '%page.html.twig' AND e.kind = 'imports'"
    ).unwrap()
     .query_map([], |r| r.get::<_, String>(0)).unwrap()
     .flatten().collect();
    assert!(refs.contains(&"base".to_string()) || refs.iter().any(|r| r.ends_with(".base") || r == "base"),
        "missing extends ref to base, got {refs:?}");
    assert!(
        refs.iter().any(|r| r == "partials.header" || r.ends_with(".partials.header")),
        "missing include ref to partials.header, got {refs:?}"
    );
}

#[test]
fn blade_and_twig_files_produce_nonzero_symbols() {
    // E2 acceptance #5 — every .blade.php / .twig produces non-zero
    // symbol count, even if it's just the host file symbol.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("resources/views")).unwrap();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(root.join("resources/views/empty.blade.php"), "<h1>x</h1>").unwrap();
    fs::write(root.join("templates/empty.html.twig"), "<h1>x</h1>").unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let blade_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE f.path LIKE '%empty.blade.php'",
        [], |r| r.get(0),
    ).unwrap();
    assert!(blade_count > 0, "Blade file should have a host symbol");

    let twig_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE f.path LIKE '%empty.html.twig'",
        [], |r| r.get(0),
    ).unwrap();
    assert!(twig_count > 0, "Twig file should have a host symbol");
}
