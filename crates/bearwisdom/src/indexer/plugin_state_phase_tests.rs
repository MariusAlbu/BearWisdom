use std::collections::HashMap;
use std::path::PathBuf;

use super::*;
use crate::db::Database;
use crate::languages::robot::RobotExternalSources;
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

/// A refresh call with `robot_external_sources: None` must not clobber a
/// `RobotExternalSources` slot an earlier call already populated — the
/// caller passes `None` precisely when it has no fresh sources to report
/// (a post-materialization plugin-state refresh, not a Robot-owning pass).
#[test]
fn populate_post_externals_none_preserves_existing_robot_sources() {
    let registry = crate::languages::default_registry();
    let mut ctx = ProjectContext::default();
    let mut sources = RobotExternalSources::default();
    sources
        .abs_by_virtual
        .insert("ext:py:seleniumlibrary".to_string(), PathBuf::from("/seed"));
    ctx.plugin_state.set(sources);

    populate_post_externals(registry, &mut ctx, &[], Path::new("."), None);

    let preserved = ctx
        .plugin_state
        .get::<RobotExternalSources>()
        .expect("RobotExternalSources must survive a None refresh call");
    assert_eq!(
        preserved.abs_by_virtual.get("ext:py:seleniumlibrary"),
        Some(&PathBuf::from("/seed")),
    );
}

/// A refresh call with `Some(sources)` sets the slot, mirroring the
/// original single-call behavior before the `Option` wrap.
#[test]
fn populate_post_externals_some_sets_robot_sources() {
    let registry = crate::languages::default_registry();
    let mut ctx = ProjectContext::default();
    let mut sources = RobotExternalSources::default();
    sources
        .abs_by_virtual
        .insert("ext:py:builtin".to_string(), PathBuf::from("/robot"));

    populate_post_externals(registry, &mut ctx, &[], Path::new("."), Some(sources));

    let stored = ctx
        .plugin_state
        .get::<RobotExternalSources>()
        .expect("Some(sources) must set the slot");
    assert_eq!(
        stored.abs_by_virtual.get("ext:py:builtin"),
        Some(&PathBuf::from("/robot")),
    );
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
    let mut symbol_id_map: SymbolIdMap = HashMap::new();
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
