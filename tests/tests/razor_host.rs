//! Integration tests for E1 — Razor host.
//!
//! Verifies that `.cshtml` files go through the Razor region detector and
//! emit C# + JS symbols that land in the index with the correct
//! `origin_language` attribution.

use std::fs;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

#[test]
fn cshtml_code_block_produces_csharp_symbols() {
    // @code { ... } and @functions { ... } contain real C# method and
    // field declarations. These must round-trip through the embedded
    // pipeline as csharp symbols on the Razor host file.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("Views")).unwrap();
    fs::write(
        root.join("Views/Index.cshtml"),
        r#"<h1>Hello</h1>

@code {
    public int Count { get; set; }
    public void Increment() {
        Count++;
    }
}

<p>Done</p>
"#,
    ).unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // Confirm file was indexed as razor.
    let lang: String = db.query_row(
        "SELECT language FROM files WHERE path LIKE '%Index.cshtml'",
        [], |r| r.get(0),
    ).expect("Index.cshtml row missing");
    assert_eq!(lang, "razor");

    // Confirm at least one csharp-origin symbol was produced.
    let csharp_symbols: Vec<String> = db.prepare(
        "SELECT s.name FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE f.path LIKE '%Index.cshtml'
           AND s.origin_language = 'csharp'
         ORDER BY s.name"
    ).unwrap()
     .query_map([], |r| r.get::<_, String>(0)).unwrap()
     .flatten().collect();

    assert!(
        csharp_symbols.iter().any(|n| n == "Increment"),
        "expected csharp symbol 'Increment' from @code block, got {csharp_symbols:?}"
    );
}

#[test]
fn cshtml_script_block_produces_js_symbols() {
    // <script>...</script> inside a .cshtml must be dispatched to the
    // JS extractor, with symbols tagged origin_language='javascript'.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("Views")).unwrap();
    fs::write(
        root.join("Views/Page.cshtml"),
        r#"<h1>x</h1>
<script>
function onReady() {
    document.title = "Hello";
}
function onSubmit() {}
</script>
"#,
    ).unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let js_symbols: Vec<String> = db.prepare(
        "SELECT s.name FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE f.path LIKE '%Page.cshtml'
           AND s.origin_language = 'javascript'
         ORDER BY s.name"
    ).unwrap()
     .query_map([], |r| r.get::<_, String>(0)).unwrap()
     .flatten().collect();

    assert!(
        js_symbols.iter().any(|n| n == "onReady"),
        "expected js symbol 'onReady' from <script>, got {js_symbols:?}"
    );
    assert!(
        js_symbols.iter().any(|n| n == "onSubmit"),
        "expected js symbol 'onSubmit' from <script>, got {js_symbols:?}"
    );
}

#[test]
fn cshtml_model_directive_surfaces_type_ref() {
    // @model produces a ref (and/or a synthetic field symbol) whose type
    // is the declared model. The synthetic wrapper is stripped — user
    // never sees `__RazorBody` in qualified_name.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("Views")).unwrap();
    fs::write(
        root.join("Views/User.cshtml"),
        "@model MyApp.Models.UserViewModel\n<h1>Hello</h1>\n",
    ).unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // Any csharp-origin symbol on User.cshtml must not leak the
    // __RazorBody prefix.
    let leaked: Vec<String> = db.prepare(
        "SELECT s.qualified_name FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE f.path LIKE '%User.cshtml'
           AND s.qualified_name LIKE '__RazorBody%'"
    ).unwrap()
     .query_map([], |r| r.get::<_, String>(0)).unwrap()
     .flatten().collect();
    assert!(leaked.is_empty(), "synthetic prefix leaked: {leaked:?}");

    // `UserViewModel` must appear somewhere as a type reference — either
    // an unresolved_ref (no csproj) or an external_ref. Search both.
    let type_referenced: i64 = db.query_row(
        "SELECT (SELECT COUNT(*) FROM unresolved_refs ur
                 JOIN symbols s ON s.id = ur.source_id
                 JOIN files   f ON f.id = s.file_id
                 WHERE f.path LIKE '%User.cshtml'
                   AND ur.target_name = 'UserViewModel')
              + (SELECT COUNT(*) FROM external_refs er
                 JOIN symbols s ON s.id = er.source_id
                 JOIN files   f ON f.id = s.file_id
                 WHERE f.path LIKE '%User.cshtml'
                   AND er.target_name = 'UserViewModel')",
        [], |r| r.get(0),
    ).unwrap();
    assert!(
        type_referenced > 0,
        "@model payload UserViewModel did not surface as a type ref"
    );
}

#[test]
fn cshtml_if_control_flow_surfaces_refs() {
    // @if (user.IsAdmin) { ... } — `user` is referenced in the condition.
    // Must produce a csharp-origin ref.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("Views")).unwrap();
    fs::write(
        root.join("Views/Gate.cshtml"),
        r#"@code { public bool IsAdmin() { return false; } }
@if (IsAdmin()) { <p>Admin</p> }
"#,
    ).unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // The @code block produces a method `IsAdmin`; the @if block calls
    // it. We should see a resolved Calls edge between them.
    let calls: i64 = db.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols src ON src.id = e.source_id
         JOIN symbols tgt ON tgt.id = e.target_id
         JOIN files f ON f.id = src.file_id
         WHERE f.path LIKE '%Gate.cshtml'
           AND tgt.name = 'IsAdmin'
           AND e.kind = 'calls'",
        [], |r| r.get(0),
    ).unwrap();
    assert!(
        calls > 0,
        "expected Calls edge from @if body to IsAdmin() method in @code"
    );
}

#[test]
fn cshtml_no_wrapper_prefix_leaks_to_qualified_names() {
    // Regression guard: __RazorBody must never appear in any symbol's
    // qualified_name or scope_path for .cshtml files.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("Views")).unwrap();
    fs::write(
        root.join("Views/All.cshtml"),
        r#"@model Foo
@using Bar.Baz
@code {
    public int Count { get; set; }
    public void Increment() { Count++; }
}
@if (Count > 0) { <p>yes</p> }
"#,
    ).unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let leak: i64 = db.query_row(
        "SELECT COUNT(*) FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE f.path LIKE '%All.cshtml'
           AND (s.qualified_name LIKE '__RazorBody%'
                OR s.scope_path LIKE '__RazorBody%')",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(leak, 0, "synthetic __RazorBody prefix must be stripped");
}

#[test]
fn cshtml_mixed_content_produces_both_languages() {
    // One Razor file with both a @code block AND a <script> block —
    // the L1 language breakdown should surface both csharp and javascript
    // as present even though neither is the host language.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("Views")).unwrap();
    fs::write(
        root.join("Views/Mix.cshtml"),
        r#"<h1>x</h1>
@code {
    public void ServerSide() {}
}
<script>
function clientSide() {}
</script>
"#,
    ).unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let ov = bearwisdom::query::architecture::get_overview(&db).unwrap();
    let has_csharp = ov.languages.iter().any(|l| l.language == "csharp" && l.symbol_count > 0);
    let has_javascript = ov.languages.iter().any(|l| l.language == "javascript" && l.symbol_count > 0);
    assert!(has_csharp, "architecture overview missing csharp: {:?}", ov.languages);
    assert!(has_javascript, "architecture overview missing javascript: {:?}", ov.languages);
}
