// Sibling test file for `unresolved_classify.rs`.

use super::*;
use crate::db::Database;

fn empty_externals() -> HashSet<String> {
    HashSet::new()
}

#[test]
fn extractor_garbage_punctuation() {
    let cat = _test_classify_row(
        "foo()",
        "calls",
        None,
        "src/a.ts",
        "typescript",
        &empty_externals(),
        None,
    );
    assert_eq!(cat, UnresolvedCategory::ExtractorBug);
}

#[test]
fn extractor_garbage_empty() {
    let cat = _test_classify_row(
        "",
        "calls",
        None,
        "src/a.ts",
        "typescript",
        &empty_externals(),
        None,
    );
    assert_eq!(cat, UnresolvedCategory::ExtractorBug);
}

#[test]
fn extractor_keyword_and_literal() {
    for (name, lang) in [
        ("if", "typescript"),
        ("return", "rust"),
        ("true", "python"),
        ("None", "python"),
        ("self", "python"),
        ("new", "csharp"),
        ("42", "rust"),
    ] {
        let cat = _test_classify_row(
            name,
            "calls",
            None,
            "src/a.ts",
            lang,
            &empty_externals(),
            None,
        );
        assert_eq!(
            cat,
            UnresolvedCategory::ExtractorBug,
            "expected {name} ({lang}) to be ExtractorBug, got {cat:?}"
        );
    }
}

#[test]
fn generated_path_node_modules() {
    let cat = _test_classify_row(
        "Foo",
        "type_ref",
        None,
        "node_modules/some-pkg/dist/index.d.ts",
        "typescript",
        &empty_externals(),
        None,
    );
    assert_eq!(cat, UnresolvedCategory::GeneratedOrVendorNoise);
}

#[test]
fn generated_filename_suffix() {
    let cat = _test_classify_row(
        "Foo",
        "type_ref",
        None,
        "src/Models.designer.cs",
        "csharp",
        &empty_externals(),
        None,
    );
    assert_eq!(cat, UnresolvedCategory::GeneratedOrVendorNoise);
}

#[test]
fn module_resolution_miss_via_module_column() {
    let cat = _test_classify_row(
        "thing",
        "calls",
        Some("./missing"),
        "src/a.ts",
        "typescript",
        &empty_externals(),
        None,
    );
    assert_eq!(cat, UnresolvedCategory::ModuleResolutionMiss);
}

#[test]
fn module_resolution_miss_via_imports_table() {
    let mut imports = HashSet::new();
    imports.insert("MyComponent".to_string());
    let cat = _test_classify_row(
        "MyComponent",
        "type_ref",
        None,
        "src/a.ts",
        "typescript",
        &empty_externals(),
        Some(&imports),
    );
    assert_eq!(cat, UnresolvedCategory::ModuleResolutionMiss);
}

#[test]
fn external_api_unknown_via_external_refs() {
    let mut externals = HashSet::new();
    externals.insert("Observable".to_string());
    let cat = _test_classify_row(
        "Observable",
        "type_ref",
        None,
        "src/a.ts",
        "typescript",
        &externals,
        None,
    );
    assert_eq!(cat, UnresolvedCategory::ExternalApiUnknown);
}

#[test]
fn local_false_positive_lowercase_short() {
    let cat = _test_classify_row(
        "i",
        "reads",
        None,
        "src/a.ts",
        "typescript",
        &empty_externals(),
        None,
    );
    assert_eq!(cat, UnresolvedCategory::LocalFalsePositive);
}

#[test]
fn local_false_positive_lowercase_word() {
    let cat = _test_classify_row(
        "result",
        "calls",
        None,
        "src/a.ts",
        "typescript",
        &empty_externals(),
        None,
    );
    assert_eq!(cat, UnresolvedCategory::LocalFalsePositive);
}

#[test]
fn local_false_positive_does_not_fire_on_dotted() {
    let cat = _test_classify_row(
        "ctx.runfiles",
        "calls",
        None,
        "src/a.bzl",
        "starlark",
        &empty_externals(),
        None,
    );
    // Dotted lowercase falls through past the locals check; with no other
    // signals the fallback is RealMissingSymbol.
    assert_ne!(cat, UnresolvedCategory::LocalFalsePositive);
}

#[test]
fn local_false_positive_does_not_fire_on_inherits() {
    let cat = _test_classify_row(
        "base",
        "inherits",
        None,
        "src/a.ts",
        "typescript",
        &empty_externals(),
        None,
    );
    // Inheritance with a lowercase target is not a local — it's a real
    // missing symbol or a module miss, but never a locals.scm leak.
    assert_ne!(cat, UnresolvedCategory::LocalFalsePositive);
}

#[test]
fn unsupported_syntax_generic_residue() {
    let cat = _test_classify_row(
        "Foo<T>",
        "type_ref",
        None,
        "src/a.ts",
        "typescript",
        &empty_externals(),
        None,
    );
    assert_eq!(cat, UnresolvedCategory::UnsupportedSyntax);
}

#[test]
fn embedded_region_capitalized_target_in_vue_host() {
    let cat = _test_classify_row(
        "OnMounted",
        "calls",
        None,
        "src/Foo.vue",
        "vue",
        &empty_externals(),
        None,
    );
    assert_eq!(cat, UnresolvedCategory::EmbeddedRegionIssue);
}

#[test]
fn real_missing_symbol_fallback() {
    let cat = _test_classify_row(
        "MyCustomType",
        "type_ref",
        None,
        "src/a.ts",
        "typescript",
        &empty_externals(),
        None,
    );
    assert_eq!(cat, UnresolvedCategory::RealMissingSymbol);
}

#[test]
fn priority_extractor_bug_beats_module_miss() {
    // Even with an explicit module, syntactic garbage is still tagged
    // as extractor_bug — fixing the resolver wouldn't help.
    let cat = _test_classify_row(
        "foo()",
        "calls",
        Some("./bar"),
        "src/a.ts",
        "typescript",
        &empty_externals(),
        None,
    );
    assert_eq!(cat, UnresolvedCategory::ExtractorBug);
}

#[test]
fn priority_module_miss_beats_external_api() {
    let mut externals = HashSet::new();
    externals.insert("Observable".to_string());
    let cat = _test_classify_row(
        "Observable",
        "type_ref",
        Some("rxjs"),
        "src/a.ts",
        "typescript",
        &externals,
        None,
    );
    // module IS NOT NULL means we identified the import path; external
    // catch-all should NOT win here.
    assert_eq!(cat, UnresolvedCategory::ModuleResolutionMiss);
}

// ---------------------------------------------------------------------------
// End-to-end against an in-memory DB
// ---------------------------------------------------------------------------

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

fn seed_symbol(db: &Database, file_id: i64, name: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, origin)
             VALUES (?1, ?2, ?3, 'function', 1, 0, 'internal')",
            rusqlite::params![file_id, name, format!("mod::{name}")],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

fn seed_unresolved(
    db: &Database,
    source_id: i64,
    target_name: &str,
    kind: &str,
    module: Option<&str>,
    line: u32,
) {
    db.conn()
        .execute(
            "INSERT INTO unresolved_refs (source_id, target_name, kind, source_line, module, from_snippet)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            rusqlite::params![source_id, target_name, kind, line, module],
        )
        .unwrap();
}

#[test]
fn report_groups_and_samples() {
    let db = Database::open_in_memory().unwrap();
    let f = seed_file(&db, "src/a.ts", "typescript", "internal");
    let s = seed_symbol(&db, f, "caller");

    seed_unresolved(&db, s, "i", "reads", None, 10);
    seed_unresolved(&db, s, "j", "reads", None, 11);
    seed_unresolved(&db, s, "MyType", "type_ref", None, 12);
    seed_unresolved(&db, s, "Observable", "type_ref", None, 13);

    db.conn()
        .execute(
            "INSERT INTO external_refs (source_id, target_name, kind, source_line, namespace)
             VALUES (?1, 'Observable', 'type_ref', 1, 'rxjs')",
            [s],
        )
        .unwrap();

    let report = classify_unresolved(&db, 5).unwrap();
    assert_eq!(report.total, 4);
    assert_eq!(report.by_language.get("typescript").copied(), Some(4));
    assert_eq!(report.by_category.get("local_false_positive").copied(), Some(2));
    assert_eq!(report.by_category.get("external_api_unknown").copied(), Some(1));
    assert_eq!(report.by_category.get("real_missing_symbol").copied(), Some(1));

    // Top-N samples carry per-target counts.
    let local_samples = report.samples.get("typescript.local_false_positive").unwrap();
    let names: Vec<&str> = local_samples.iter().map(|s| s.target_name.as_str()).collect();
    assert!(names.contains(&"i"));
    assert!(names.contains(&"j"));
}

#[test]
fn report_excludes_markdown_doc_links() {
    // Markdown link refs (kind=imports) are doc cross-references, not
    // code resolution failures — they must not appear in the classifier
    // total nor any bucket.
    let db = Database::open_in_memory().unwrap();
    let f_md = seed_file(&db, "README.md", "markdown", "internal");
    let f_ts = seed_file(&db, "src/a.ts", "typescript", "internal");
    let s_md = seed_symbol(&db, f_md, "README");
    let s_ts = seed_symbol(&db, f_ts, "caller");

    seed_unresolved(&db, s_md, "doc/Other", "imports", None, 1);
    seed_unresolved(&db, s_ts, "Foo", "type_ref", None, 1);

    let report = classify_unresolved(&db, 5).unwrap();
    assert_eq!(report.total, 1);
    assert!(report.by_language.get("markdown").is_none());
    assert_eq!(report.by_language.get("typescript").copied(), Some(1));
}

#[test]
fn report_excludes_external_origin_and_snippets() {
    let db = Database::open_in_memory().unwrap();
    let f_int = seed_file(&db, "src/a.ts", "typescript", "internal");
    let f_ext = seed_file(&db, "ext/pkg/x.d.ts", "typescript", "external");
    let s_int = seed_symbol(&db, f_int, "caller");
    let s_ext = seed_symbol(&db, f_ext, "extcaller");

    seed_unresolved(&db, s_int, "Foo", "type_ref", None, 1);
    // External-origin source — must NOT count.
    db.conn()
        .execute(
            "UPDATE symbols SET origin = 'external' WHERE id = ?1",
            [s_ext],
        )
        .unwrap();
    seed_unresolved(&db, s_ext, "Bar", "type_ref", None, 1);
    // Snippet — must NOT count.
    db.conn()
        .execute(
            "INSERT INTO unresolved_refs (source_id, target_name, kind, source_line, from_snippet)
             VALUES (?1, 'Snip', 'calls', 1, 1)",
            [s_int],
        )
        .unwrap();

    let report = classify_unresolved(&db, 5).unwrap();
    assert_eq!(report.total, 1);
}
