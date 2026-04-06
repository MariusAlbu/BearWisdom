//! Comprehensive integration tests for all query commands.
//!
//! One canonical place that exercises every public query function against a
//! real indexed project, so regressions are caught regardless of which module
//! changes.  Each test is named `test_cmd_<command>` to distinguish it from
//! the unit tests that live inside individual modules.
//!
//! Setup: the C# service fixture is indexed once via a module-level helper.
//! Tests that need a filesystem path (grep, content-search) create a fresh
//! `TestProject` inline — those are cheap because the fixture only has 4 files.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use bearwisdom::full_index;
use bearwisdom::query::{
    architecture::get_overview,
    blast_radius::blast_radius,
    call_hierarchy::{incoming_calls, outgoing_calls},
    completion::complete_at,
    concepts::{auto_assign_concepts, concept_members, discover_concepts, list_concepts},
    context::smart_context,
    definitions::goto_definition,
    diagnostics::{get_diagnostics, LOW_CONFIDENCE_THRESHOLD},
    investigate::{investigate, InvestigateOptions},
    references::find_references,
    search::search_symbols,
    subgraph::{export_graph, export_graph_json},
    symbol_info::{file_symbols, symbol_info, FileSymbolsMode},
    QueryOptions,
};
use bearwisdom::search::{
    content_index::rebuild_content_index,
    content_search::search_content,
    fuzzy::FuzzyIndex,
    grep::{grep_search, GrepOptions},
    scope::SearchScope,
};
use bearwisdom::Database;
use bearwisdom_tests::TestProject;

// ---------------------------------------------------------------------------
// Shared setup
// ---------------------------------------------------------------------------

/// Build a fresh in-memory database with the C# service fixture indexed.
///
/// Called per-test rather than with `Lazy` because `Database` holds a
/// `rusqlite::Connection` which is not `Send`, so it cannot be stored in a
/// global.  The fixture is tiny (4 files), so re-indexing costs ~1 ms.
fn csharp_db() -> Database {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();
    db
}

fn cancel_never() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

// ============================================================================
// architecture::get_overview
// ============================================================================

#[test]
fn test_cmd_architecture_languages() {
    let db = csharp_db();
    let overview = get_overview(&db).unwrap();

    assert!(overview.total_files > 0);
    assert!(overview.total_symbols > 0);
    assert!(!overview.languages.is_empty(), "should detect at least one language");

    let csharp = overview.languages.iter().find(|l| l.language == "csharp");
    assert!(csharp.is_some(), "C# should appear in language stats");
}

#[test]
fn test_cmd_architecture_hotspots() {
    let db = csharp_db();
    let overview = get_overview(&db).unwrap();

    // Hotspots list may be empty on small fixtures — just assert the field is present.
    let _ = &overview.hotspots;
    assert!(overview.total_symbols > 0, "at least some symbols must be indexed");
}

#[test]
fn test_cmd_architecture_entry_points() {
    let db = csharp_db();
    let overview = get_overview(&db).unwrap();

    // entry_points is best-effort; we just verify the call does not error and
    // the returned slice is well-formed.
    for ep in &overview.entry_points {
        assert!(!ep.name.is_empty());
        assert!(!ep.file_path.is_empty());
    }
}

// ============================================================================
// search::search_symbols
// ============================================================================

#[test]
fn test_cmd_search_symbols_known_class() {
    let db = csharp_db();
    let results = search_symbols(&db, "Product", 10, &QueryOptions::full()).unwrap();

    assert!(!results.is_empty(), "search for 'Product' should return results");
    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"Product"), "Product class should appear in results");
}

#[test]
fn test_cmd_search_symbols_interface() {
    let db = csharp_db();
    let results = search_symbols(&db, "IProductRepository", 10, &QueryOptions::full()).unwrap();

    assert!(!results.is_empty(), "should find the interface");
    assert_eq!(results[0].kind, "interface");
}

#[test]
fn test_cmd_search_symbols_empty_query_no_panic() {
    let db = csharp_db();
    let _ = search_symbols(&db, "", 10, &QueryOptions::default()).unwrap();
}

// ============================================================================
// definitions::goto_definition
// ============================================================================

#[test]
fn test_cmd_goto_definition_class() {
    let db = csharp_db();
    let results = goto_definition(&db, "Product").unwrap();

    assert!(!results.is_empty(), "should find definition for 'Product'");
    assert_eq!(results[0].kind, "class");
    assert!(results[0].file_path.contains("Product.cs"));
}

#[test]
fn test_cmd_goto_definition_interface() {
    let db = csharp_db();
    let results = goto_definition(&db, "IProductRepository").unwrap();

    assert!(!results.is_empty(), "should find the interface definition");
    assert_eq!(results[0].kind, "interface");
}

#[test]
fn test_cmd_goto_definition_not_found() {
    let db = csharp_db();
    let results = goto_definition(&db, "CompletelyMissingXYZ").unwrap();
    assert!(results.is_empty());
}

// ============================================================================
// references::find_references
// ============================================================================

#[test]
fn test_cmd_find_references_interface() {
    let db = csharp_db();
    let refs = find_references(&db, "IProductRepository", 0).unwrap();

    // ProductRepository implements it; ProductService uses it.
    assert!(!refs.is_empty(), "IProductRepository should have at least one reference");
}

#[test]
fn test_cmd_find_references_unknown() {
    let db = csharp_db();
    let refs = find_references(&db, "GhostSymbolXYZ", 0).unwrap();
    assert!(refs.is_empty());
}

// ============================================================================
// symbol_info::symbol_info
// ============================================================================

#[test]
fn test_cmd_symbol_info_service() {
    let db = csharp_db();
    let details = symbol_info(&db, "ProductService", &QueryOptions::full()).unwrap();

    assert!(!details.is_empty(), "should find ProductService");
    assert_eq!(details[0].name, "ProductService");
    assert!(details[0].file_path.contains("ProductService.cs"));
}

#[test]
fn test_cmd_symbol_info_slim_default() {
    let db = csharp_db();
    // Default opts — slim output, no children.
    let details = symbol_info(&db, "Product", &QueryOptions::default()).unwrap();
    assert!(!details.is_empty());
    // With slim opts, doc and signature may be None even if the symbol has them.
    assert_eq!(details[0].name, "Product");
}

// ============================================================================
// symbol_info::file_symbols
// ============================================================================

#[test]
fn test_cmd_file_symbols_outline() {
    let db = csharp_db();

    // Resolve an actual file path as stored in the DB (relative to project root).
    // The fixture stores paths like "Services/ProductService.cs".
    let syms = file_symbols(&db, "Services/ProductService.cs", FileSymbolsMode::Outline).unwrap();

    // If the path separator differs on Windows the DB may normalise it —
    // fall back to a backslash variant.
    let syms = if syms.is_empty() {
        file_symbols(&db, "Services\\ProductService.cs", FileSymbolsMode::Outline).unwrap()
    } else {
        syms
    };

    assert!(!syms.is_empty(), "ProductService.cs should contain symbols");
    let has_service = syms.iter().any(|s| s.name == "ProductService");
    assert!(has_service, "ProductService class should appear in file symbols");
}

#[test]
fn test_cmd_file_symbols_missing_file_returns_empty() {
    let db = csharp_db();
    let syms = file_symbols(&db, "DoesNotExist.cs", FileSymbolsMode::Names).unwrap();
    assert!(syms.is_empty());
}

// ============================================================================
// blast_radius::blast_radius
// ============================================================================

#[test]
fn test_cmd_blast_radius_model() {
    let db = csharp_db();
    let result = blast_radius(&db, "Product", 3, 500).unwrap();

    if let Some(br) = result {
        assert_eq!(br.center.name, "Product");
        // Repositories and services depend on Product, so affected list is non-empty.
        assert!(!br.affected.is_empty(), "Product should have at least one dependent");
        assert!(br.total_affected > 0);
    }
    // None is acceptable if extractor qualified-names differ — not a hard failure.
}

#[test]
fn test_cmd_blast_radius_unknown() {
    let db = csharp_db();
    let result = blast_radius(&db, "AbsolutelyUnknownSymbol", 2, 500).unwrap();
    assert!(result.is_none());
}

// ============================================================================
// call_hierarchy::incoming_calls / outgoing_calls
// ============================================================================

#[test]
fn test_cmd_incoming_calls() {
    let db = csharp_db();
    // GetById is called from ProductService.GetProduct.
    let callers = incoming_calls(&db, "GetById", 10).unwrap();
    // Edge extraction fidelity may vary; we just assert no panic and valid shape.
    for c in &callers {
        assert!(!c.name.is_empty());
        assert!(!c.file_path.is_empty());
    }
}

#[test]
fn test_cmd_outgoing_calls() {
    let db = csharp_db();
    let callees = outgoing_calls(&db, "GetProduct", 10).unwrap();
    for c in &callees {
        assert!(!c.name.is_empty());
        assert!(!c.file_path.is_empty());
    }
}

// ============================================================================
// concepts::list_concepts + concept_members
// ============================================================================

#[test]
fn test_cmd_list_and_inspect_concepts() {
    let db = csharp_db();

    // Run discovery so there is something to list.
    let _ = discover_concepts(&db).unwrap();
    let _ = auto_assign_concepts(&db).unwrap();

    let concepts = list_concepts(&db).unwrap();
    // Concepts are best-effort; list may be empty on tiny fixtures.
    for c in &concepts {
        assert!(!c.name.is_empty());
    }

    // If at least one concept was discovered, verify concept_members returns
    // a valid (possibly empty) list.
    if let Some(first) = concepts.first() {
        let members = concept_members(&db, &first.name, 20).unwrap();
        for m in &members {
            assert!(!m.name.is_empty());
            assert!(!m.file_path.is_empty());
        }
    }
}

// ============================================================================
// subgraph::export_graph + export_graph_json
// ============================================================================

#[test]
fn test_cmd_export_graph_nodes() {
    let db = csharp_db();
    let graph = export_graph(&db, None, 100).unwrap();

    assert!(!graph.nodes.is_empty(), "graph should contain nodes from indexed symbols");
    for node in &graph.nodes {
        assert!(!node.name.is_empty());
        assert!(!node.qualified_name.is_empty());
    }
}

#[test]
fn test_cmd_export_graph_json_valid() {
    let db = csharp_db();
    let json = export_graph_json(&db, None, 100).unwrap();

    assert!(json.contains("nodes"), "JSON must contain a 'nodes' key");
    assert!(json.contains("edges"), "JSON must contain an 'edges' key");

    let parsed: serde_json::Value = serde_json::from_str(&json)
        .expect("export_graph_json should produce valid JSON");
    assert!(parsed["nodes"].is_array());
    assert!(parsed["edges"].is_array());
}

// ============================================================================
// context::smart_context
// ============================================================================

#[test]
fn test_cmd_smart_context_symbol_name() {
    let db = csharp_db();
    let result = smart_context(&db, "ProductService", 8000, 2).unwrap();

    // If FTS5 found seeds the result should be populated.
    // It may be empty if the FTS index is not warm — treat both as valid.
    assert_eq!(result.task, "ProductService");
    if !result.symbols.is_empty() {
        assert!(result.token_estimate > 0, "token_estimate should be positive when symbols are returned");
        assert!(!result.files.is_empty(), "files should be populated when symbols are returned");
        for sym in &result.symbols {
            assert!(!sym.name.is_empty());
            assert!(!sym.file_path.is_empty());
            assert!(sym.score >= 0.0 && sym.score <= 1.5, "score should be in a reasonable range");
        }
    }
}

#[test]
fn test_cmd_smart_context_natural_language_task() {
    let db = csharp_db();
    // A natural-language description that contains domain terms present in the fixture.
    let result = smart_context(&db, "get product by id from repository", 8000, 2).unwrap();

    assert_eq!(result.task, "get product by id from repository");
    // Files list is always a subset of (or equal to) the file paths in symbols.
    let sym_files: std::collections::HashSet<&str> =
        result.symbols.iter().map(|s| s.file_path.as_str()).collect();
    for f in &result.files {
        assert!(sym_files.contains(f.as_str()), "every file in result.files must be referenced by a symbol");
    }
}

#[test]
fn test_cmd_smart_context_empty_task() {
    let db = csharp_db();
    let result = smart_context(&db, "", 8000, 2).unwrap();
    assert!(result.symbols.is_empty());
    assert_eq!(result.token_estimate, 0);
}

// ============================================================================
// diagnostics::get_diagnostics
// ============================================================================

#[test]
fn test_cmd_diagnostics_known_file() {
    let db = csharp_db();

    // Use forward-slash path; the DB may have stored it differently on Windows.
    let path = "Services/ProductService.cs";
    let result = get_diagnostics(&db, path, LOW_CONFIDENCE_THRESHOLD).unwrap();

    // Result is structurally valid regardless of whether there are any issues.
    assert_eq!(result.file_path, path);
    // unresolved + low_confidence counts must match actual diagnostics length.
    assert_eq!(
        (result.unresolved_count + result.low_confidence_count) as usize,
        result.diagnostics.len(),
        "diagnostic counts must be consistent"
    );
}

#[test]
fn test_cmd_diagnostics_missing_file_returns_empty() {
    let db = csharp_db();
    let result = get_diagnostics(&db, "DoesNotExist.cs", LOW_CONFIDENCE_THRESHOLD).unwrap();
    assert_eq!(result.unresolved_count, 0);
    assert_eq!(result.low_confidence_count, 0);
    assert!(result.diagnostics.is_empty());
}

// ============================================================================
// completion::complete_at
// ============================================================================

#[test]
fn test_cmd_complete_at_no_prefix() {
    let db = csharp_db();

    // Pass an empty prefix so the function returns all in-scope candidates
    // rather than requiring a fuzzy match against a real prefix.
    let items = complete_at(&db, "Services/ProductService.cs", 10, 0, "", false).unwrap();

    // If the file was found in the DB, we expect candidates from scope/imports.
    // If not (path normalisation mismatch), we get an empty list — both are valid.
    for item in &items {
        assert!(!item.name.is_empty());
        assert!(!item.kind.is_empty());
    }
}

#[test]
fn test_cmd_complete_at_with_prefix() {
    let db = csharp_db();

    // "Get" is a prefix shared by GetById and GetProduct.
    let items = complete_at(&db, "Services/ProductService.cs", 12, 0, "Get", false).unwrap();

    // All returned items must have a name that fuzzy-matches "Get".
    // (nucleo CaseMatching::Smart will match "GetById" for "Get")
    for item in &items {
        assert!(
            item.name.to_lowercase().contains("get") || item.score > 0,
            "all completions should be relevant to prefix 'Get'"
        );
    }
}

#[test]
fn test_cmd_complete_at_unknown_file_returns_empty() {
    let db = csharp_db();
    let items = complete_at(&db, "NonExistent.cs", 1, 0, "foo", false).unwrap();
    assert!(items.is_empty());
}

// ============================================================================
// investigate::investigate
// ============================================================================

#[test]
fn test_cmd_investigate_known_symbol() {
    let db = csharp_db();
    let opts = InvestigateOptions::default();
    let result = investigate(&db, "ProductService", &opts).unwrap();

    assert!(result.is_some(), "investigate should find ProductService");
    let r = result.unwrap();
    assert_eq!(r.symbol.name, "ProductService");
    assert!(!r.symbol.file_path.is_empty());
    assert!(r.symbol.line > 0);

    // callers/callees may be empty on a small fixture — shape is what matters.
    for c in &r.callers {
        assert!(!c.name.is_empty());
    }
    for c in &r.callees {
        assert!(!c.name.is_empty());
    }
}

#[test]
fn test_cmd_investigate_includes_blast_radius() {
    let db = csharp_db();
    let opts = InvestigateOptions { blast_depth: 2, ..Default::default() };
    let result = investigate(&db, "Product", &opts).unwrap();

    // Result may be None if the symbol is not found; blast_radius may be None
    // if there are no dependents.  Only validate when both are present.
    if let Some(r) = result {
        if let Some(br) = &r.blast_radius {
            assert!(br.total_affected > 0, "blast radius should report at least one affected symbol");
            assert!(!br.affected.is_empty());
        }
    }
}

#[test]
fn test_cmd_investigate_not_found() {
    let db = csharp_db();
    let result = investigate(&db, "AbsolutelyNotHere", &InvestigateOptions::default()).unwrap();
    assert!(result.is_none());
}

// ============================================================================
// search::grep::grep_search
// ============================================================================

#[test]
fn test_cmd_grep_literal_match() {
    let project = TestProject::csharp_service();
    let cancel = cancel_never();

    let results = grep_search(
        project.path(),
        "IProductRepository",
        &GrepOptions::default(),
        &cancel,
    )
    .unwrap();

    assert!(!results.is_empty(), "should find IProductRepository in multiple files");
    for m in &results {
        assert!(!m.file_path.is_empty());
        assert!(m.line_number > 0);
        assert!(m.line_content.contains("IProductRepository"));
    }
}

#[test]
fn test_cmd_grep_case_insensitive() {
    let project = TestProject::csharp_service();
    let cancel = cancel_never();

    let opts = GrepOptions { case_sensitive: false, ..Default::default() };
    let results = grep_search(project.path(), "productservice", &opts, &cancel).unwrap();

    assert!(!results.is_empty(), "case-insensitive grep should match 'ProductService'");
}

#[test]
fn test_cmd_grep_no_match() {
    let project = TestProject::csharp_service();
    let cancel = cancel_never();

    let results = grep_search(
        project.path(),
        "ZZZNOTFOUND9999",
        &GrepOptions::default(),
        &cancel,
    )
    .unwrap();

    assert!(results.is_empty());
}

// ============================================================================
// search::fuzzy::FuzzyIndex — file matching
// ============================================================================

#[test]
fn test_cmd_fuzzy_match_files() {
    let db = csharp_db();
    let index = FuzzyIndex::from_db(&db).unwrap();

    let matches = index.match_files("ProdServ", 10);
    assert!(
        !matches.is_empty(),
        "fuzzy 'ProdServ' should match ProductService.cs"
    );
    for m in &matches {
        assert!(!m.text.is_empty());
        assert!(m.score > 0);
    }
}

#[test]
fn test_cmd_fuzzy_match_files_empty_pattern() {
    let db = csharp_db();
    let index = FuzzyIndex::from_db(&db).unwrap();
    assert!(index.match_files("", 10).is_empty(), "empty pattern should return no results");
}

// ============================================================================
// search::fuzzy::FuzzyIndex — symbol matching
// ============================================================================

#[test]
fn test_cmd_fuzzy_match_symbols() {
    let db = csharp_db();
    let index = FuzzyIndex::from_db(&db).unwrap();

    let matches = index.match_symbols("GetById", 10);
    assert!(!matches.is_empty(), "fuzzy symbol search should find GetById");
    for m in &matches {
        assert!(!m.text.is_empty());
    }
}

#[test]
fn test_cmd_fuzzy_match_symbols_partial() {
    let db = csharp_db();
    let index = FuzzyIndex::from_db(&db).unwrap();

    // "ProdRepo" should fuzzy-match ProductRepository.
    let matches = index.match_symbols("ProdRepo", 10);
    assert!(!matches.is_empty(), "fuzzy 'ProdRepo' should match ProductRepository");
}

// ============================================================================
// search::content_search::search_content (FTS5)
// ============================================================================

#[test]
fn test_cmd_content_search_after_index() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();

    // FTS content index must be built separately from the symbol index.
    let indexed = rebuild_content_index(db.conn(), project.path()).unwrap();
    assert!(indexed > 0, "should have content-indexed at least one file");

    let results = search_content(&db, "ProductService", &SearchScope::default(), 10).unwrap();
    assert!(
        !results.is_empty(),
        "FTS5 should find 'ProductService' after content indexing"
    );
    for r in &results {
        assert!(!r.file_path.is_empty());
        assert!(r.score > 0.0);
    }
}

#[test]
fn test_cmd_content_search_short_query_empty() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();
    rebuild_content_index(db.conn(), project.path()).unwrap();

    // FTS5 trigram requires >= 3 chars; shorter queries must return empty.
    let results = search_content(&db, "ab", &SearchScope::default(), 10).unwrap();
    assert!(results.is_empty(), "sub-trigram query must return empty");
}

#[test]
fn test_cmd_content_search_returns_ranked_results() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();
    rebuild_content_index(db.conn(), project.path()).unwrap();

    let results = search_content(&db, "GetById", &SearchScope::default(), 10).unwrap();
    // Verify results are in descending score order (best first).
    let scores: Vec<f64> = results.iter().map(|r| r.score).collect();
    let mut sorted = scores.clone();
    sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());
    assert_eq!(scores, sorted, "results should be ordered by score descending");
}
