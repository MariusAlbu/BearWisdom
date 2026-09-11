use super::*;
use crate::db::Database;
use crate::type_checker::core::types::TypeArena;
use crate::types::{FlowMeta, ParsedFile};

fn empty_parsed_file(path: &str, language: &str) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: language.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

/// No active plugin in `language_presence` means no `synthesize_project_symbols`
/// hook runs, so the phase reports `false` (nothing synthesized) and touches
/// neither the DB nor `symbol_id_map` — the signal a caller uses to skip a
/// tree rebuild.
#[test]
fn synthesize_and_persist_returns_false_when_nothing_synthesized() {
    let mut db = Database::open_in_memory().unwrap();
    let ctx = ProjectContext::default(); // language_presence is empty
    let mut parsed = vec![empty_parsed_file("src/a.ex", "elixir")];
    let mut symbol_id_map = crate::indexer::write::SymbolIds::default();
    let arena = TypeArena::new();

    let synthesized = synthesize_and_persist(
        crate::languages::default_registry(),
        &ctx,
        &mut parsed,
        &mut db,
        &mut symbol_id_map,
        &arena,
    )
    .unwrap();

    assert!(!synthesized);
    assert!(symbol_id_map.is_empty());
}
