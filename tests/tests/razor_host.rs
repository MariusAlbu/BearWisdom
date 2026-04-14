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
