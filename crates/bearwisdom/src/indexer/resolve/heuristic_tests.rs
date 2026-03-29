use super::*;
use crate::types::{ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind};

fn make_parsed_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "csharp".to_string(),
        content_hash: "abc".to_string(),
        size: 100,
        line_count: 10,
        symbols,
        refs,
        routes: vec![],
        db_sets: vec![],
        content: None,
        has_errors: false,
    }
}

fn make_sym(name: &str, qname: &str) -> ExtractedSymbol {
    make_sym_kind(name, qname, SymbolKind::Method)
}

fn make_sym_kind(name: &str, qname: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: None,
        start_line: 1,
        end_line: 5,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

#[test]
fn name_index_built_correctly() {
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("a.cs".to_string(), "NS.Foo.Bar".to_string()), 42);
    id_map.insert(("b.cs".to_string(), "Other.Bar".to_string()), 99);

    let idx = build_name_index(&id_map, &[]);
    let bar_entries = idx.get("Bar").unwrap();
    assert_eq!(bar_entries.len(), 2);
    let ids: Vec<i64> = bar_entries.iter().map(|(_, _, _, id)| *id).collect();
    assert!(ids.contains(&42));
    assert!(ids.contains(&99));
}

#[test]
fn qname_lookup_works() {
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("a.cs".to_string(), "NS.Foo.GetById".to_string()), 7);
    let qmap = build_qname_index(&id_map);
    assert_eq!(qmap.get("NS.Foo.GetById"), Some(&7));
}

#[test]
fn file_path_matches_relative_ts_import() {
    assert!(file_path_matches_module("src/api/catalog.ts", "./catalog"));
    assert!(file_path_matches_module("src/api/catalog.ts", "catalog"));
    assert!(!file_path_matches_module("src/api/catalog.ts", "./orders"));
}

// WP-3: P1.5 namespace import resolver
//
// File A: `using NS;` declares `NS.Foo` is available.
// File B: defines `NS.Foo` as a class.
// A TypeRef to "Foo" in file A should resolve at confidence 0.92.
#[test]
fn p1_5_namespace_import_resolves_at_0_92() {
    use crate::types::EdgeKind;

    // File A: has `using NS;` and a method that TypeRefs to "Foo".
    let sym_a = make_sym("DoWork", "MyApp.MyClass.DoWork");
    let ref_import = ExtractedRef {
        source_symbol_index: 0,
        target_name: "NS".to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        module: Some("NS".to_string()),
        chain: None,
    };
    let ref_type = ExtractedRef {
        source_symbol_index: 0,
        target_name: "Foo".to_string(),
        kind: EdgeKind::TypeRef,
        line: 5,
        module: None,
        chain: None,
    };
    let file_a = make_parsed_file("a.cs", vec![sym_a], vec![ref_import, ref_type]);

    // File B: defines `NS.Foo`.
    let sym_b = make_sym_kind("Foo", "NS.Foo", SymbolKind::Class);
    let file_b = make_parsed_file("b.cs", vec![sym_b], vec![]);

    let parsed = vec![file_a, file_b];

    // Build qname_to_id as the resolver does.
    let mut symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    symbol_id_map.insert(("a.cs".to_string(), "MyApp.MyClass.DoWork".to_string()), 1);
    symbol_id_map.insert(("b.cs".to_string(), "NS.Foo".to_string()), 2);

    let _qname_to_id = build_qname_index(&symbol_id_map);
    let _file_imports = build_import_map(&parsed);

    // "NS" has no dots so P1.5 should NOT fire for single-segment import.
    // Instead build a dotted import scenario.
    let dotted_imports: Vec<(String, Option<String>)> = vec![
        ("NS.Models".to_string(), Some("NS.Models".to_string())),
    ];

    // Register NS.Models.Foo in qname_to_id.
    let mut qname_to_id2: HashMap<String, i64> = HashMap::new();
    qname_to_id2.insert("NS.Models.Foo".to_string(), 42);

    let result = resolve_via_namespace_import("Foo", &dotted_imports, &qname_to_id2);
    assert_eq!(result, Some(42), "P1.5 should resolve NS.Models.Foo via dotted import");

    // Single-segment import should NOT resolve via P1.5.
    let single_imports: Vec<(String, Option<String>)> = vec![
        ("System".to_string(), Some("System".to_string())),
    ];
    let result2 = resolve_via_namespace_import("Foo", &single_imports, &qname_to_id2);
    assert_eq!(result2, None, "P1.5 should skip single-segment imports");

    // Verify the full resolve pipeline: using "NS.Models" with TypeRef to "Foo"
    // should yield (42, 0.92).
    let name_to_ids = build_name_index(&symbol_id_map, &parsed);
    let source_file = "a.cs";
    let resolution = resolve_ref(
        "Foo",
        EdgeKind::TypeRef,
        source_file,
        &dotted_imports,
        None,
        &name_to_ids,
        &qname_to_id2,
        &symbol_id_map,
        &parsed,
    );
    assert_eq!(
        resolution,
        Some((42, 0.92)),
        "Full pipeline: TypeRef to 'Foo' with using NS.Models should resolve at 0.92"
    );
}

// WP-7: P4 kind matching
//
// A `Calls` ref to "Foo" should prefer a method symbol over a class symbol
// of the same name.
#[test]
fn p4_kind_matching_prefers_method_for_calls() {
    use crate::types::EdgeKind;

    // Two symbols named "Foo": one class, one method.
    let sym_class = make_sym_kind("Foo", "NS.Foo", SymbolKind::Class);
    let sym_method = make_sym_kind("Foo", "Other.Foo", SymbolKind::Method);
    let file_a = make_parsed_file("a.cs", vec![sym_class], vec![]);
    let file_b = make_parsed_file("b.cs", vec![sym_method], vec![]);
    let parsed = vec![file_a, file_b];

    let mut symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
    symbol_id_map.insert(("a.cs".to_string(), "NS.Foo".to_string()), 10); // class
    symbol_id_map.insert(("b.cs".to_string(), "Other.Foo".to_string()), 20); // method

    let name_to_ids = build_name_index(&symbol_id_map, &parsed);
    let qname_to_id = build_qname_index(&symbol_id_map);

    // A Calls ref should pick the method (id=20) over the class (id=10).
    let resolution = resolve_ref(
        "Foo",
        EdgeKind::Calls,
        "caller.cs",
        &[],
        None,
        &name_to_ids,
        &qname_to_id,
        &symbol_id_map,
        &parsed,
    );
    assert_eq!(
        resolution.map(|(id, _)| id),
        Some(20),
        "Calls ref to 'Foo' should prefer the method symbol over the class symbol"
    );
}

// WP-7: kind_matches_symbol_kind correctness
#[test]
fn kind_matches_logic_is_correct() {
    assert!(kind_matches_symbol_kind(EdgeKind::Calls, "method"));
    assert!(kind_matches_symbol_kind(EdgeKind::Calls, "function"));
    assert!(kind_matches_symbol_kind(EdgeKind::Calls, "constructor"));
    assert!(!kind_matches_symbol_kind(EdgeKind::Calls, "class"));
    assert!(!kind_matches_symbol_kind(EdgeKind::Calls, "interface"));

    assert!(kind_matches_symbol_kind(EdgeKind::Inherits, "class"));
    assert!(kind_matches_symbol_kind(EdgeKind::Inherits, "struct"));
    assert!(!kind_matches_symbol_kind(EdgeKind::Inherits, "interface"));
    assert!(!kind_matches_symbol_kind(EdgeKind::Inherits, "method"));

    assert!(kind_matches_symbol_kind(EdgeKind::Implements, "interface"));
    assert!(!kind_matches_symbol_kind(EdgeKind::Implements, "class"));

    assert!(kind_matches_symbol_kind(EdgeKind::TypeRef, "class"));
    assert!(kind_matches_symbol_kind(EdgeKind::TypeRef, "enum"));
    assert!(kind_matches_symbol_kind(EdgeKind::TypeRef, "delegate"));
    assert!(!kind_matches_symbol_kind(EdgeKind::TypeRef, "method"));

    // Imports accepts any kind.
    assert!(kind_matches_symbol_kind(EdgeKind::Imports, "method"));
    assert!(kind_matches_symbol_kind(EdgeKind::Imports, "class"));
}
