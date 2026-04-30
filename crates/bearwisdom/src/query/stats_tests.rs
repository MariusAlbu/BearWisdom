// Sibling test file for `stats.rs`. Covers the doc-cross-reference
// filter that was added so markdown/mdx link refs don't drag down the
// code resolution metric.

use super::*;
use crate::db::Database;

fn open() -> Database {
    Database::open_in_memory().unwrap()
}

fn seed_file(db: &Database, path: &str, language: &str, origin: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed, origin)
             VALUES (?1, 'h', ?2, 0, ?3)",
            rusqlite::params![path, language, origin],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

fn seed_symbol(db: &Database, file_id: i64, name: &str, origin: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, origin)
             VALUES (?1, ?2, ?3, 'function', 1, 0, ?4)",
            rusqlite::params![file_id, name, format!("mod::{name}"), origin],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

fn seed_unresolved(
    db: &Database,
    source_id: i64,
    target_name: &str,
    kind: &str,
    from_snippet: u8,
) {
    db.conn()
        .execute(
            "INSERT INTO unresolved_refs (source_id, target_name, kind, source_line, from_snippet)
             VALUES (?1, ?2, ?3, 1, ?4)",
            rusqlite::params![source_id, target_name, kind, from_snippet],
        )
        .unwrap();
}

#[test]
fn resolution_breakdown_excludes_markdown_imports() {
    let db = open();
    let f_md = seed_file(&db, "README.md", "markdown", "internal");
    let f_ts = seed_file(&db, "src/a.ts", "typescript", "internal");
    let s_md = seed_symbol(&db, f_md, "README", "internal");
    let s_ts = seed_symbol(&db, f_ts, "caller", "internal");

    // Doc cross-reference — must NOT count.
    seed_unresolved(&db, s_md, "doc/Other", "imports", 0);
    seed_unresolved(&db, s_md, "guides/setup", "imports", 0);
    // Real code-resolution miss — must count.
    seed_unresolved(&db, s_ts, "MissingType", "type_ref", 0);

    let rb = resolution_breakdown(&db).unwrap();
    assert_eq!(rb.internal_unresolved, 1, "expected only the TS row to count");
    assert!(rb.unresolved_by_lang_kind.get("markdown.imports").is_none());
    assert_eq!(rb.unresolved_by_lang_kind.get("typescript.type_ref").copied(), Some(1));
}

#[test]
fn resolution_breakdown_excludes_mdx_imports() {
    let db = open();
    let f = seed_file(&db, "docs/index.mdx", "mdx", "internal");
    let s = seed_symbol(&db, f, "index", "internal");
    seed_unresolved(&db, s, "../guides/A", "imports", 0);

    let rb = resolution_breakdown(&db).unwrap();
    assert_eq!(rb.internal_unresolved, 0);
}

#[test]
fn resolution_breakdown_keeps_mdx_calls() {
    // mdx kind=calls is the embedded-region issue — it IS a code-resolution
    // failure (the JSX inside MDX should resolve through the TS resolver)
    // and must still count toward the metric.
    let db = open();
    let f = seed_file(&db, "docs/index.mdx", "mdx", "internal");
    let s = seed_symbol(&db, f, "index", "internal");
    seed_unresolved(&db, s, "useState", "calls", 0);

    let rb = resolution_breakdown(&db).unwrap();
    assert_eq!(rb.internal_unresolved, 1);
    assert_eq!(rb.unresolved_by_lang_kind.get("mdx.calls").copied(), Some(1));
}

#[test]
fn resolution_breakdown_keeps_snippet_filter() {
    // from_snippet=1 was already excluded; the new filter doesn't
    // change that behavior.
    let db = open();
    let f = seed_file(&db, "src/a.ts", "typescript", "internal");
    let s = seed_symbol(&db, f, "caller", "internal");
    seed_unresolved(&db, s, "Snip", "calls", 1); // from snippet
    seed_unresolved(&db, s, "Real", "calls", 0); // not from snippet

    let rb = resolution_breakdown(&db).unwrap();
    assert_eq!(rb.internal_unresolved, 1);
}

#[test]
fn index_stats_internal_unresolved_excludes_doc_links() {
    let db = open();
    let f_md = seed_file(&db, "README.md", "markdown", "internal");
    let f_ts = seed_file(&db, "src/a.ts", "typescript", "internal");
    let s_md = seed_symbol(&db, f_md, "README", "internal");
    let s_ts = seed_symbol(&db, f_ts, "caller", "internal");

    seed_unresolved(&db, s_md, "doc/Other", "imports", 0);
    seed_unresolved(&db, s_ts, "Foo", "type_ref", 0);

    let stats = index_stats(&db).unwrap();
    assert_eq!(stats.unresolved_ref_count, 1);
}
