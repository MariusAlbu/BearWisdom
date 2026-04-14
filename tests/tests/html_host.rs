//! Integration tests for E6 — Plain HTML host.
//!
//! Verifies inline `<script>` and `<style>` blocks in `.html` files
//! dispatch correctly through the indexer.

use std::fs;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

#[test]
fn inline_script_produces_javascript_symbols() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("index.html"),
        r#"<!doctype html>
<html>
<body>
<script>
function greet(name) {
    return "hi " + name;
}
</script>
</body>
</html>
"#,
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%index.html'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "html");

    let js_syms: Vec<String> = db
        .prepare(
            "SELECT s.name FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%index.html'
               AND s.origin_language = 'javascript'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        js_syms.iter().any(|n| n == "greet"),
        "expected JS 'greet' from inline <script>, got {js_syms:?}"
    );
}

#[test]
fn inline_typescript_script_produces_typescript_symbols() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("page.html"),
        r#"<html><body>
<script lang="ts">
export function farewell(name: string): string {
    return "bye " + name;
}
</script>
</body></html>
"#,
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let ts_syms: Vec<String> = db
        .prepare(
            "SELECT s.name FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%page.html'
               AND s.origin_language = 'typescript'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        ts_syms.iter().any(|n| n == "farewell"),
        "expected TS 'farewell' from <script lang=ts>, got {ts_syms:?}"
    );
}

#[test]
fn json_typed_script_is_skipped() {
    // `<script type="application/ld+json">` must NOT produce JS symbols
    // or trigger parser errors — it's structured data, not code.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("data.html"),
        r#"<html><body>
<script type="application/ld+json">
{ "@context": "https://schema.org", "@type": "Person", "name": "Jane" }
</script>
</body></html>
"#,
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // No JS-origin symbols because the JSON block should be skipped.
    let js_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%data.html'
               AND s.origin_language = 'javascript'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(js_count, 0);
}

#[test]
fn element_ids_become_anchor_symbols() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("page.html"),
        r#"<html><body>
<section id="intro">Hi</section>
<section id="usage">Use.</section>
</body></html>
"#,
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let anchor_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%page.html'
               AND s.kind = 'field'
               AND s.scope_path = 'page'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(anchor_count, 2);
}
