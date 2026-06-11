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

fn seed_unresolved(db: &Database, source_id: i64, target_name: &str, kind: &str, from_snippet: u8) {
    db.conn()
        .execute(
            "INSERT INTO unresolved_refs (source_id, target_name, kind, source_line, from_snippet)
             VALUES (?1, ?2, ?3, 1, ?4)",
            rusqlite::params![source_id, target_name, kind, from_snippet],
        )
        .unwrap();
}

fn seed_external(db: &Database, source_id: i64, target_name: &str, kind: &str, namespace: &str) {
    db.conn()
        .execute(
            "INSERT INTO external_refs (source_id, target_name, kind, source_line, namespace)
             VALUES (?1, ?2, ?3, 1, ?4)",
            rusqlite::params![source_id, target_name, kind, namespace],
        )
        .unwrap();
}

fn seed_edge(db: &Database, source_id: i64, target_id: i64) {
    // Distinct source_line per insert so repeated edges between the same
    // (source, target, kind) pair don't collide on the unique index.
    use std::sync::atomic::{AtomicI64, Ordering};
    static NEXT_LINE: AtomicI64 = AtomicI64::new(1);
    let source_line = NEXT_LINE.fetch_add(1, Ordering::Relaxed);
    db.conn()
        .execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence, strategy)
             VALUES (?1, ?2, 'calls', ?3, 1.0, 'test')",
            rusqlite::params![source_id, target_id, source_line],
        )
        .unwrap();
}

#[test]
fn resolution_breakdown_distinguishes_three_states() {
    // The three outcomes a reference can land in must be reported as
    // separate buckets, so precision is measurable instead of hidden in
    // one coverage rate:
    //   * a real bind                  → resolved (an edge)
    //   * import names a dep, source
    //     absent on disk               → external_known_unhydrated
    //   * binds nothing, no dep owns
    //     the name (a typo)            → unresolved_unknown
    let db = open();
    let f = seed_file(&db, "src/a.ts", "typescript", "internal");
    let caller = seed_symbol(&db, f, "caller", "internal");
    let callee = seed_symbol(&db, f, "callee", "internal");

    // resolved — a bound declaration.
    seed_edge(&db, caller, callee);
    // external_known_unhydrated — scope named a real dependency whose
    // metadata was never pulled (the EXT-1 locate-miss outcome).
    seed_external(&db, caller, "findUnique", "calls", "ext:@prisma/client");
    // unresolved_unknown — the genuine gap.
    seed_unresolved(&db, caller, "frobnicate", "calls", 0);

    let rb = resolution_breakdown(&db).unwrap();

    assert_eq!(rb.internal_edges, 1, "one resolved edge");
    assert_eq!(rb.external_known_unhydrated, 1, "one dep-absent ref");
    assert_eq!(rb.internal_unresolved, 1, "one genuine unknown");

    // Precision counts only resolved vs unknown — the unhydrated dep is
    // tracked apart and must NOT drag the denominator down.
    // 1 / (1 + 1) = 50%.
    assert_eq!(rb.precision, 50.0);
    assert_eq!(rb.precision, rb.internal_resolution_rate);
}

#[test]
fn external_known_unhydrated_excluded_from_precision_denominator() {
    // A project whose every miss is a known-but-unhydrated dependency has
    // perfect precision — the engine bound everything it could see; the
    // gaps are dependency-availability, not resolution failures.
    let db = open();
    let f = seed_file(&db, "src/a.ts", "typescript", "internal");
    let caller = seed_symbol(&db, f, "caller", "internal");
    let callee = seed_symbol(&db, f, "callee", "internal");

    seed_edge(&db, caller, callee);
    seed_external(
        &db,
        caller,
        "useQuery",
        "calls",
        "ext:@tanstack/react-query",
    );
    seed_external(&db, caller, "axios", "calls", "ext:axios");

    let rb = resolution_breakdown(&db).unwrap();

    assert_eq!(rb.internal_edges, 1);
    assert_eq!(rb.external_known_unhydrated, 2);
    assert_eq!(rb.internal_unresolved, 0);
    assert_eq!(
        rb.precision, 100.0,
        "unhydrated deps must not count as failures"
    );
}

fn seed_symbol_origin_lang(
    db: &Database,
    file_id: i64,
    name: &str,
    origin_language: &str,
) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO symbols
                (file_id, name, qualified_name, kind, line, col, origin, origin_language)
             VALUES (?1, ?2, ?3, 'function', 1, 0, 'internal', ?4)",
            rusqlite::params![file_id, name, format!("mod::{name}"), origin_language],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

#[test]
fn rate_by_language_splits_two_languages() {
    // Two languages with different resolved/unresolved ratios must each
    // get their own edge total and rate; a language present on only one
    // side still appears.
    let db = open();
    let f_ts = seed_file(&db, "src/a.ts", "typescript", "internal");
    let f_go = seed_file(&db, "src/b.go", "go", "internal");

    let ts_caller = seed_symbol(&db, f_ts, "tsCaller", "internal");
    let ts_callee = seed_symbol(&db, f_ts, "tsCallee", "internal");
    let go_caller = seed_symbol(&db, f_go, "goCaller", "internal");
    let go_callee = seed_symbol(&db, f_go, "goCallee", "internal");

    // typescript: 3 resolved edges, 1 unresolved → 75.00%.
    seed_edge(&db, ts_caller, ts_callee);
    seed_edge(&db, ts_caller, ts_callee);
    seed_edge(&db, ts_caller, ts_callee);
    seed_unresolved(&db, ts_caller, "tsMissing", "calls", 0);

    // go: 1 resolved edge, 3 unresolved → 25.00%.
    seed_edge(&db, go_caller, go_callee);
    seed_unresolved(&db, go_caller, "goMiss1", "calls", 0);
    seed_unresolved(&db, go_caller, "goMiss2", "type_ref", 0);
    seed_unresolved(&db, go_caller, "goMiss3", "calls", 0);

    let rb = resolution_breakdown(&db).unwrap();

    assert_eq!(rb.internal_edges_by_lang.get("typescript").copied(), Some(3));
    assert_eq!(rb.internal_edges_by_lang.get("go").copied(), Some(1));

    assert_eq!(rb.rate_by_language.get("typescript").copied(), Some(75.0));
    assert_eq!(rb.rate_by_language.get("go").copied(), Some(25.0));

    // Headline pooled rate is the corpus-wide value, distinct from either
    // per-language rate: 4 edges / (4 + 4) = 50%.
    assert_eq!(rb.internal_edges, 4);
    assert_eq!(rb.internal_unresolved, 4);
    assert_eq!(rb.resolution_rate, 50.0);
}

#[test]
fn rate_by_language_present_on_one_side_only() {
    // A language with edges but no unresolved refs scores 100.0; one with
    // only unresolved refs scores 0.0.
    let db = open();
    let f_rs = seed_file(&db, "src/a.rs", "rust", "internal");
    let f_py = seed_file(&db, "src/b.py", "python", "internal");
    let rs_caller = seed_symbol(&db, f_rs, "rsCaller", "internal");
    let rs_callee = seed_symbol(&db, f_rs, "rsCallee", "internal");
    let py_caller = seed_symbol(&db, f_py, "pyCaller", "internal");

    seed_edge(&db, rs_caller, rs_callee);
    seed_unresolved(&db, py_caller, "pyMissing", "calls", 0);

    let rb = resolution_breakdown(&db).unwrap();

    assert_eq!(rb.rate_by_language.get("rust").copied(), Some(100.0));
    assert_eq!(rb.rate_by_language.get("python").copied(), Some(0.0));
    assert_eq!(rb.internal_edges_by_lang.get("rust").copied(), Some(1));
    assert_eq!(rb.internal_edges_by_lang.get("python"), None);
}

#[test]
fn rate_by_language_attributes_via_origin_language() {
    // The load-bearing case: a symbol whose `origin_language` differs from
    // its file's language is attributed to `origin_language`, not the file
    // language — so a C codebase indexed under a non-C project name reports
    // its edges and rate under C.
    let db = open();
    let f = seed_file(&db, "src/perl_internals.c", "perl", "internal");
    let caller = seed_symbol_origin_lang(&db, f, "cCaller", "c");
    let callee = seed_symbol_origin_lang(&db, f, "cCallee", "c");

    seed_edge(&db, caller, callee);
    seed_unresolved(&db, caller, "cMissing", "calls", 0);

    let rb = resolution_breakdown(&db).unwrap();

    // Attribution follows origin_language: the rate lands under "c", and the
    // file's nominal "perl" language carries nothing.
    assert_eq!(rb.internal_edges_by_lang.get("c").copied(), Some(1));
    assert_eq!(rb.rate_by_language.get("c").copied(), Some(50.0));
    assert_eq!(rb.internal_edges_by_lang.get("perl"), None);
    assert_eq!(rb.rate_by_language.get("perl"), None);
}

#[test]
fn rate_by_language_two_decimals() {
    // 1 edge, 2 unresolved → 33.33%, matching the two-decimal rounding the
    // headline rate uses.
    let db = open();
    let f = seed_file(&db, "src/a.ts", "typescript", "internal");
    let caller = seed_symbol(&db, f, "caller", "internal");
    let callee = seed_symbol(&db, f, "callee", "internal");
    seed_edge(&db, caller, callee);
    seed_unresolved(&db, caller, "m1", "calls", 0);
    seed_unresolved(&db, caller, "m2", "calls", 0);

    let rb = resolution_breakdown(&db).unwrap();
    assert_eq!(rb.rate_by_language.get("typescript").copied(), Some(33.33));
}

#[test]
fn rate_by_language_excludes_doc_link_refs() {
    // The denominator must honor CODE_REF_FILTER: markdown `imports` refs
    // are doc cross-references, not resolution failures, and must not drag a
    // language's rate down. With only a doc-link miss, markdown has no
    // counted unresolved ref, so it stays off the map entirely.
    let db = open();
    let f_md = seed_file(&db, "README.md", "markdown", "internal");
    let s_md = seed_symbol(&db, f_md, "README", "internal");
    seed_unresolved(&db, s_md, "doc/Other", "imports", 0);

    let rb = resolution_breakdown(&db).unwrap();
    assert_eq!(rb.rate_by_language.get("markdown"), None);
    assert_eq!(rb.internal_edges_by_lang.get("markdown"), None);
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
    assert_eq!(
        rb.internal_unresolved, 1,
        "expected only the TS row to count"
    );
    assert!(rb.unresolved_by_lang_kind.get("markdown.imports").is_none());
    assert_eq!(
        rb.unresolved_by_lang_kind
            .get("typescript.type_ref")
            .copied(),
        Some(1)
    );
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
    assert_eq!(
        rb.unresolved_by_lang_kind.get("mdx.calls").copied(),
        Some(1)
    );
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

// ---------------------------------------------------------------------------
// flow_diagnostics tests
// ---------------------------------------------------------------------------

fn seed_flow_edge(
    db: &Database,
    source_file_id: i64,
    source_line: i64,
    target_file_id: Option<i64>,
    edge_type: &str,
    protocol: Option<&str>,
    url_pattern: Option<&str>,
    source_language: Option<&str>,
) {
    db.conn()
        .execute(
            "INSERT INTO flow_edges
                (source_file_id, source_line, target_file_id,
                 edge_type, protocol, url_pattern, source_language, confidence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0.9)",
            rusqlite::params![
                source_file_id,
                source_line,
                target_file_id,
                edge_type,
                protocol,
                url_pattern,
                source_language,
            ],
        )
        .unwrap();
}

#[test]
fn flow_diagnostics_empty_index() {
    let db = open();
    let report = flow_diagnostics(&db).unwrap();
    assert_eq!(report.total, 0);
    assert_eq!(report.paired, 0);
    assert_eq!(report.single_ended, 0);
    assert_eq!(report.pairing_rate, 100.0);
    assert!(report.by_edge_type.is_empty());
    assert!(report.top_single_ended.is_empty());
}

#[test]
fn flow_diagnostics_counts_paired_vs_single_ended() {
    let db = open();
    let f1 = seed_file(&db, "src/a.ts", "typescript", "internal");
    let f2 = seed_file(&db, "src/b.cs", "csharp", "internal");

    // Two paired (target_file_id present), three single-ended. Distinct
    // source_lines so the unique-index dedup doesn't collapse them.
    seed_flow_edge(
        &db,
        f1,
        10,
        Some(f2),
        "http_call",
        Some("rest"),
        Some("/api/x"),
        Some("typescript"),
    );
    seed_flow_edge(
        &db,
        f1,
        11,
        Some(f2),
        "http_call",
        Some("rest"),
        Some("/api/y"),
        Some("typescript"),
    );
    seed_flow_edge(
        &db,
        f1,
        12,
        None,
        "http_call",
        Some("rest"),
        Some("/api/missing"),
        Some("typescript"),
    );
    seed_flow_edge(
        &db,
        f1,
        13,
        None,
        "http_call",
        Some("rest"),
        Some("/api/missing"),
        Some("typescript"),
    );
    seed_flow_edge(
        &db,
        f1,
        14,
        None,
        "rpc_call",
        Some("grpc"),
        Some("UserService/Get"),
        Some("typescript"),
    );

    let report = flow_diagnostics(&db).unwrap();

    assert_eq!(report.total, 5);
    assert_eq!(report.paired, 2);
    assert_eq!(report.single_ended, 3);
    assert_eq!(report.pairing_rate, 40.0);

    // by_edge_type sorted by single_ended desc: http_call (2 single) before rpc_call (1 single).
    assert_eq!(report.by_edge_type.len(), 2);
    assert_eq!(report.by_edge_type[0].edge_type, "http_call");
    assert_eq!(report.by_edge_type[0].paired, 2);
    assert_eq!(report.by_edge_type[0].single_ended, 2);
    assert_eq!(report.by_edge_type[0].pairing_rate, 50.0);
    assert_eq!(report.by_edge_type[1].edge_type, "rpc_call");
    assert_eq!(report.by_edge_type[1].paired, 0);
    assert_eq!(report.by_edge_type[1].single_ended, 1);
}

#[test]
fn flow_diagnostics_groups_single_ended_examples() {
    let db = open();
    let f1 = seed_file(&db, "src/a.ts", "typescript", "internal");

    // Three single-ended rows sharing one URL — should collapse to one
    // worklist entry with count=3.
    seed_flow_edge(
        &db,
        f1,
        10,
        None,
        "http_call",
        Some("rest"),
        Some("/api/users"),
        Some("typescript"),
    );
    seed_flow_edge(
        &db,
        f1,
        11,
        None,
        "http_call",
        Some("rest"),
        Some("/api/users"),
        Some("typescript"),
    );
    seed_flow_edge(
        &db,
        f1,
        12,
        None,
        "http_call",
        Some("rest"),
        Some("/api/users"),
        Some("typescript"),
    );
    // One single-ended row with a different URL — separate entry.
    seed_flow_edge(
        &db,
        f1,
        13,
        None,
        "http_call",
        Some("rest"),
        Some("/api/orders"),
        Some("typescript"),
    );

    let report = flow_diagnostics(&db).unwrap();

    assert_eq!(report.top_single_ended.len(), 2);
    assert_eq!(
        report.top_single_ended[0].url_pattern.as_deref(),
        Some("/api/users")
    );
    assert_eq!(report.top_single_ended[0].count, 3);
    assert_eq!(
        report.top_single_ended[1].url_pattern.as_deref(),
        Some("/api/orders")
    );
    assert_eq!(report.top_single_ended[1].count, 1);
}

#[test]
fn flow_diagnostics_by_source_language() {
    let db = open();
    let f_ts = seed_file(&db, "src/a.ts", "typescript", "internal");
    let f_cs = seed_file(&db, "src/b.cs", "csharp", "internal");
    let f_target = seed_file(&db, "src/c.go", "go", "internal");

    seed_flow_edge(
        &db,
        f_ts,
        10,
        Some(f_target),
        "http_call",
        Some("rest"),
        Some("/a"),
        Some("typescript"),
    );
    seed_flow_edge(
        &db,
        f_ts,
        11,
        None,
        "http_call",
        Some("rest"),
        Some("/b"),
        Some("typescript"),
    );
    seed_flow_edge(
        &db,
        f_cs,
        12,
        None,
        "rpc_call",
        Some("grpc"),
        Some("Svc/M"),
        Some("csharp"),
    );

    let report = flow_diagnostics(&db).unwrap();

    let ts = report.by_source_language.get("typescript").unwrap();
    assert_eq!(ts.paired, 1);
    assert_eq!(ts.single_ended, 1);
    assert_eq!(ts.pairing_rate(), 50.0);

    let cs = report.by_source_language.get("csharp").unwrap();
    assert_eq!(cs.paired, 0);
    assert_eq!(cs.single_ended, 1);
    assert_eq!(cs.pairing_rate(), 0.0);
}

#[test]
fn flow_pairing_rate_handles_empty_bucket() {
    let p = FlowPairing {
        paired: 0,
        single_ended: 0,
    };
    assert_eq!(p.pairing_rate(), 100.0);
}

#[test]
fn flow_pairing_rate_rounds_to_two_decimals() {
    let p = FlowPairing {
        paired: 1,
        single_ended: 2,
    };
    assert_eq!(p.pairing_rate(), 33.33);
}
