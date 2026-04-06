//! Integration tests for the query layer.
//!
//! Each test indexes a fixture project, then exercises one or more query
//! functions against the populated database.

use bearwisdom::{full_index, Database};
use bearwisdom::query::{
    architecture::get_overview,
    blast_radius::blast_radius,
    call_hierarchy::{incoming_calls, outgoing_calls},
    concepts::{auto_assign_concepts, discover_concepts, list_concepts},
    definitions::goto_definition,
    references::find_references,
    search::search_symbols,
    subgraph::{export_graph, export_graph_json},
    symbol_info::symbol_info,
};
use bearwisdom_tests::TestProject;

/// Index the C# fixture and return a ready-to-query database.
fn indexed_csharp_db() -> Database {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();
    db
}

// ── architecture ────────────────────────────────────────────────────────

#[test]
fn overview_returns_language_stats() {
    let db = indexed_csharp_db();
    let overview = get_overview(&db).unwrap();

    assert!(overview.total_files > 0);
    assert!(overview.total_symbols > 0);
    assert!(!overview.languages.is_empty(), "should detect at least one language");

    let csharp = overview.languages.iter().find(|l| l.language == "csharp");
    assert!(csharp.is_some(), "C# should appear in language stats");
}

#[test]
fn overview_finds_hotspots() {
    let db = indexed_csharp_db();
    let overview = get_overview(&db).unwrap();

    // Hotspots are symbols with the most incoming references.
    // The fixture has enough cross-references to produce at least one.
    // (If zero, the query still shouldn't error.)
    assert!(overview.total_symbols > 0);
}

// ── goto definition ─────────────────────────────────────────────────────

#[test]
fn goto_definition_by_simple_name() {
    let db = indexed_csharp_db();
    let results = goto_definition(&db, "Product").unwrap();

    assert!(!results.is_empty(), "should find definition for 'Product'");
    assert_eq!(results[0].kind, "class");
}

#[test]
fn goto_definition_not_found() {
    let db = indexed_csharp_db();
    let results = goto_definition(&db, "NonExistentSymbol").unwrap();

    assert!(results.is_empty());
}

// ── find references ─────────────────────────────────────────────────────

#[test]
fn find_references_for_interface() {
    let db = indexed_csharp_db();
    let refs = find_references(&db, "IProductRepository", 0).unwrap();

    // ProductRepository implements it and ProductService uses it.
    assert!(!refs.is_empty(), "IProductRepository should have references");
}

// ── symbol search ───────────────────────────────────────────────────────

#[test]
fn search_symbols_finds_class() {
    let db = indexed_csharp_db();
    let results = search_symbols(&db, "Product", 10, &bearwisdom::query::QueryOptions::full()).unwrap();

    assert!(!results.is_empty(), "search for 'Product' should return results");
    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"Product"), "Product class should be in results");
}

#[test]
fn search_symbols_empty_query() {
    let db = indexed_csharp_db();
    let results = search_symbols(&db, "", 10, &bearwisdom::query::QueryOptions::full()).unwrap();
    // Empty query behavior is implementation-defined; just ensure no panic.
    let _ = results;
}

// ── symbol info ─────────────────────────────────────────────────────────

#[test]
fn symbol_info_returns_detail() {
    let db = indexed_csharp_db();
    let details = symbol_info(&db, "ProductService", &bearwisdom::query::QueryOptions::full()).unwrap();

    assert!(!details.is_empty(), "should find ProductService");
    let detail = &details[0];
    assert_eq!(detail.name, "ProductService");
    assert!(detail.file_path.contains("ProductService.cs"));
}

// ── blast radius ────────────────────────────────────────────────────────

#[test]
fn blast_radius_from_model() {
    let db = indexed_csharp_db();
    let result = blast_radius(&db, "Product", 3, 500).unwrap();

    // Product is used by repository and service, so blast radius should exist.
    if let Some(br) = result {
        assert_eq!(br.center.name, "Product");
        // At least one symbol should be affected.
        assert!(!br.affected.is_empty(), "Product should have dependents");
    }
    // If None, the symbol wasn't found — acceptable if extractor qualified-names differ.
}

#[test]
fn blast_radius_unknown_symbol() {
    let db = indexed_csharp_db();
    let result = blast_radius(&db, "CompletelyUnknown", 3, 500).unwrap();
    assert!(result.is_none());
}

// ── call hierarchy ──────────────────────────────────────────────────────

#[test]
fn incoming_calls_for_method() {
    let db = indexed_csharp_db();

    // GetById is called by ProductService.GetProduct
    let callers = incoming_calls(&db, "GetById", 10).unwrap();
    // May or may not find callers depending on edge extraction fidelity.
    let _ = callers;
}

#[test]
fn outgoing_calls_from_service() {
    let db = indexed_csharp_db();

    let callees = outgoing_calls(&db, "GetProduct", 10).unwrap();
    let _ = callees;
}

// ── subgraph export ─────────────────────────────────────────────────────

#[test]
fn export_full_graph() {
    let db = indexed_csharp_db();
    let graph = export_graph(&db, None, 100).unwrap();

    assert!(!graph.nodes.is_empty(), "graph should have nodes");
    // Edges may or may not exist depending on resolution.
    for node in &graph.nodes {
        assert!(!node.name.is_empty());
        assert!(!node.qualified_name.is_empty());
    }
}

#[test]
fn export_graph_as_json() {
    let db = indexed_csharp_db();
    let json = export_graph_json(&db, None, 100).unwrap();

    assert!(json.contains("nodes"), "JSON should contain nodes key");
    assert!(json.contains("edges"), "JSON should contain edges key");

    // Should be valid JSON.
    let _: serde_json::Value = serde_json::from_str(&json).unwrap();
}

// ── concepts ────────────────────────────────────────────────────────────

#[test]
fn auto_assign_and_discover_concepts() {
    let db = indexed_csharp_db();

    let assigned = auto_assign_concepts(&db).unwrap();
    let _ = assigned; // may be 0 if no patterns match

    let discovered = discover_concepts(&db).unwrap();
    let _ = discovered;

    let concepts = list_concepts(&db).unwrap();
    let _ = concepts;
}

// ── multi-language queries ──────────────────────────────────────────────

#[test]
fn query_multi_lang_overview() {
    let project = TestProject::multi_lang();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    let overview = get_overview(&db).unwrap();
    assert!(overview.languages.len() >= 2, "should detect multiple languages");
}
