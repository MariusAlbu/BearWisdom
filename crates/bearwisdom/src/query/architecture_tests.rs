use super::*;
use crate::db::Database;

/// Insert a file row and return its id.
fn insert_file(db: &Database, path: &str, lang: &str) -> i64 {
    db.conn().execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', ?2, 0)",
        rusqlite::params![path, lang],
    ).unwrap();
    db.conn().last_insert_rowid()
}

/// Insert a symbol row and return its id.
fn insert_symbol(db: &Database, file_id: i64, name: &str, qname: &str, kind: &str, vis: Option<&str>) -> i64 {
    db.conn().execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility)
         VALUES (?1, ?2, ?3, ?4, 1, 0, ?5)",
        rusqlite::params![file_id, name, qname, kind, vis],
    ).unwrap();
    db.conn().last_insert_rowid()
}

/// Insert a directed edge.
fn insert_edge(db: &Database, src: i64, tgt: i64, kind: &str) {
    db.conn().execute(
        "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, ?3, 1.0)",
        rusqlite::params![src, tgt, kind],
    ).unwrap();
}

#[test]
fn overview_totals_are_correct() {
    let db = Database::open_in_memory().unwrap();
    let f1 = insert_file(&db, "a.cs", "csharp");
    let s1 = insert_symbol(&db, f1, "Foo", "App.Foo", "class", Some("public"));
    let s2 = insert_symbol(&db, f1, "Bar", "App.Bar", "class", Some("public"));
    insert_edge(&db, s1, s2, "calls");

    let ov = get_overview(&db).unwrap();
    assert_eq!(ov.total_files, 1);
    assert_eq!(ov.total_symbols, 2);
    assert_eq!(ov.total_edges, 1);
}

#[test]
fn overview_language_stats() {
    let db = Database::open_in_memory().unwrap();
    let f1 = insert_file(&db, "a.cs", "csharp");
    let f2 = insert_file(&db, "b.ts", "typescript");
    insert_symbol(&db, f1, "Foo", "App.Foo", "class", None);
    insert_symbol(&db, f2, "bar", "bar", "function", None);

    let ov = get_overview(&db).unwrap();
    assert_eq!(ov.languages.len(), 2);
    // Each language should have 1 file.
    assert!(ov.languages.iter().all(|l| l.file_count == 1));
}

#[test]
fn overview_hotspots_ranked_by_incoming() {
    let db = Database::open_in_memory().unwrap();
    let f = insert_file(&db, "a.cs", "csharp");
    let popular = insert_symbol(&db, f, "Hub", "App.Hub", "class", None);
    let s1 = insert_symbol(&db, f, "A", "App.A", "method", None);
    let s2 = insert_symbol(&db, f, "B", "App.B", "method", None);
    let s3 = insert_symbol(&db, f, "C", "App.C", "method", None);
    insert_edge(&db, s1, popular, "calls");
    insert_edge(&db, s2, popular, "calls");
    insert_edge(&db, s3, popular, "type_ref");

    let ov = get_overview(&db).unwrap();
    assert!(!ov.hotspots.is_empty());
    assert_eq!(ov.hotspots[0].name, "Hub");
    assert_eq!(ov.hotspots[0].incoming_refs, 3);
}

#[test]
fn overview_entry_points_filters_public() {
    let db = Database::open_in_memory().unwrap();
    let f = insert_file(&db, "a.cs", "csharp");
    insert_symbol(&db, f, "PubClass",  "App.PubClass",  "class", Some("public"));
    insert_symbol(&db, f, "PrivClass", "App.PrivClass", "class", Some("private"));

    let ov = get_overview(&db).unwrap();
    assert_eq!(ov.entry_points.len(), 1);
    assert_eq!(ov.entry_points[0].name, "PubClass");
}

// ---------------------------------------------------------------
// L1 — embedded-region-aware language breakdown
// ---------------------------------------------------------------

/// Insert a symbol with an explicit origin_language (embedded sub-extraction).
fn insert_embedded_symbol(
    db: &Database,
    file_id: i64,
    name: &str,
    origin_lang: &str,
) -> i64 {
    db.conn().execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, origin_language)
         VALUES (?1, ?2, ?2, 'function', 1, 0, ?3)",
        rusqlite::params![file_id, name, origin_lang],
    ).unwrap();
    db.conn().last_insert_rowid()
}

#[test]
fn l1_language_breakdown_attributes_embedded_symbols_to_sublanguage() {
    // One Razor host file with 1 host symbol + 3 C# embedded symbols +
    // 2 JS embedded symbols. File count is 1 for razor (and zero for the
    // sub-languages, since they have no standalone files). Symbol counts
    // split by effective language.
    let db = Database::open_in_memory().unwrap();
    let f = insert_file(&db, "Views/Index.cshtml", "razor");
    insert_symbol(&db, f, "Model", "Views.Model", "class", None); // host razor
    insert_embedded_symbol(&db, f, "CsOne", "csharp");
    insert_embedded_symbol(&db, f, "CsTwo", "csharp");
    insert_embedded_symbol(&db, f, "CsThree", "csharp");
    insert_embedded_symbol(&db, f, "jsFn", "javascript");
    insert_embedded_symbol(&db, f, "jsFn2", "javascript");

    let ov = get_overview(&db).unwrap();

    let razor = ov.languages.iter().find(|l| l.language == "razor")
        .expect("razor row missing");
    assert_eq!(razor.file_count, 1);
    assert_eq!(razor.symbol_count, 1, "razor host symbol count wrong");

    let csharp = ov.languages.iter().find(|l| l.language == "csharp")
        .expect("csharp row missing — embedded language not surfaced");
    assert_eq!(csharp.file_count, 0, "csharp should have no standalone files");
    assert_eq!(csharp.symbol_count, 3);

    let js = ov.languages.iter().find(|l| l.language == "javascript")
        .expect("javascript row missing — embedded language not surfaced");
    assert_eq!(js.file_count, 0);
    assert_eq!(js.symbol_count, 2);
}

#[test]
fn l1_language_breakdown_single_language_no_duplicates() {
    // Regression: a pure-CSharp project with no embedded regions must
    // produce exactly one row for csharp (no accidental duplicate from
    // the UNION ALL tail when symbol_counts has the same language).
    let db = Database::open_in_memory().unwrap();
    let f = insert_file(&db, "Foo.cs", "csharp");
    insert_symbol(&db, f, "A", "App.A", "class", None);
    insert_symbol(&db, f, "B", "App.B", "class", None);

    let ov = get_overview(&db).unwrap();
    let csharp_rows: Vec<_> = ov.languages.iter()
        .filter(|l| l.language == "csharp").collect();
    assert_eq!(csharp_rows.len(), 1, "expected one csharp row, got {csharp_rows:?}");
    assert_eq!(csharp_rows[0].file_count, 1);
    assert_eq!(csharp_rows[0].symbol_count, 2);
}

#[test]
fn overview_empty_database() {
    let db = Database::open_in_memory().unwrap();
    let ov = get_overview(&db).unwrap();
    assert_eq!(ov.total_files, 0);
    assert_eq!(ov.total_symbols, 0);
    assert_eq!(ov.total_edges, 0);
    assert!(ov.languages.is_empty());
    assert!(ov.hotspots.is_empty());
    assert!(ov.entry_points.is_empty());
}
