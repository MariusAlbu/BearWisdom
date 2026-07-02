//! JavaScript resolution probes — sibling to `resolution_corpus.rs`, same
//! per-fix harness shape: one in-memory `TestProject` indexed once through
//! the real pipeline, self-validating preconditions, printed known-red
//! candidates carrying their traced root cause.
//!
//! Reuses the `BEARWISDOM_TS_LIB_DIR` stub-lib seam (same one
//! `resolution_corpus.rs` uses for TypeScript) — `ts-lib-dom` activates on
//! `LanguagePresent("javascript")` too, so a plain `.js` project needs no
//! `tsconfig.json` to pull the stub in.

use std::fs;

use bearwisdom::full_index;
use bearwisdom::Database;
use bearwisdom_tests::TestProject;
use rusqlite::params;
use tempfile::TempDir;

/// Count unresolved `calls` refs for `callee` in the file ending `file_suffix`.
fn count_unresolved(db: &Database, file_suffix: &str, callee: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM unresolved_refs u
         JOIN symbols s ON u.source_id = s.id
         JOIN files f   ON s.file_id   = f.id
         WHERE f.path LIKE ?1 AND u.kind = 'calls' AND u.target_name = ?2",
        params![format!("%{file_suffix}"), callee],
        |r| r.get(0),
    )
    .unwrap()
}

/// Count resolved `calls` edges for `callee` in the file ending `file_suffix`,
/// whose resolved target qname matches `target_like` (SQL LIKE).
fn count_resolved_to(db: &Database, file_suffix: &str, callee: &str, target_like: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols s ON e.source_id = s.id
         JOIN files f   ON s.file_id   = f.id
         JOIN symbols t ON e.target_id = t.id
         WHERE f.path LIKE ?1 AND e.kind = 'calls' AND t.name = ?2
           AND t.qualified_name LIKE ?3",
        params![format!("%{file_suffix}"), callee, target_like],
        |r| r.get(0),
    )
    .unwrap()
}

/// Seed a stub TypeScript standard library carrying just the `String.split`
/// member the corpus references — the JS-side counterpart to
/// `resolution_corpus.rs`'s `seed_ts_lib`.
fn seed_ts_lib() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("lib.es5.d.ts"),
        r#"interface String {
    split(separator: string): string[];
}
"#,
    )
    .unwrap();
    dir
}

#[test]
fn resolution_corpus_js() {
    let ts_lib = seed_ts_lib();

    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };

    // --- pattern: builtin member call on a string LITERAL receiver ----------
    // `"a,b".split(",")` — works for `.ts` (TypeScript's private
    // `build_chain_inner` boxes a `string` node to a synthetic `String` root
    // segment), dies for `.js` (the JS plugin routes through the SHARED
    // `common::build_chain_inner`, which has no literal-root arms — the
    // receiver aborts the whole chain, not just the root segment).
    project.add_file(
        "src/split_literal.js",
        r#"function useSplit() {
    return "a,b".split(",");
}
"#,
    );

    // --- pattern: JSDoc @returns-typed function, consumed by a caller -------
    // `/** @returns {Widget} */ function make() {}` then `make().render()` —
    // JSDoc comments are captured as free text only; no tag parser reads
    // `@returns` into a typed return_type.
    project.add_file(
        "src/jsdoc_return.js",
        r#"class Widget {
    render() {}
}

/**
 * @returns {Widget}
 */
function make() {
    return new Widget();
}

function useWidget() {
    make().render();
}
"#,
    );

    // Point the locator at the seeded stub, index once, restore env.
    let prior_lib = std::env::var_os("BEARWISDOM_TS_LIB_DIR");
    unsafe {
        std::env::set_var("BEARWISDOM_TS_LIB_DIR", ts_lib.path());
    }

    let mut db = TestProject::in_memory_db();
    let result = full_index(&mut db, project.path(), None, None, None);

    unsafe {
        match prior_lib {
            Some(v) => std::env::set_var("BEARWISDOM_TS_LIB_DIR", v),
            None => std::env::remove_var("BEARWISDOM_TS_LIB_DIR"),
        }
    }
    result.expect("index failed");

    // Precondition: the stub `String` interface MUST be indexed, else the
    // literal-receiver pattern's red result would be indistinguishable from
    // "stub never seeded" rather than "chain never built".
    let stub_string: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name='String' AND kind='interface' AND origin='external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!("\n--- preconditions ---");
    println!("  stub String interface indexed (BEARWISDOM_TS_LIB_DIR): {stub_string}");
    assert!(
        stub_string >= 1,
        "precondition: stub `String` interface must be indexed, else the literal-receiver pattern's \
         red result is ambiguous between a seeding gap and an extraction gap"
    );

    // Candidate probes — diagnostic only, printed regardless of outcome so a
    // fix's effect is visible as a single line flipping.
    println!("\n--- candidate probes (known red) ---");
    println!(
        "  literal-receiver  \"a,b\".split(',')  resolved-to-String={} unresolved={}",
        count_resolved_to(&db, "split_literal.js", "split", "%String%"),
        count_unresolved(&db, "split_literal.js", "split")
    );
    println!(
        "  jsdoc-return      make().render()    resolved-to-Widget={} unresolved={}",
        count_resolved_to(&db, "jsdoc_return.js", "render", "%Widget%"),
        count_unresolved(&db, "jsdoc_return.js", "render")
    );

    println!("\n=== resolution corpus (js) ===");
    println!("  0 / 0 patterns asserted — both probes above are documented-red candidates\n");
}
