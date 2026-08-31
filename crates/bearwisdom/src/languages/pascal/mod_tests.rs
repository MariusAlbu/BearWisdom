use super::PascalPlugin;
use crate::indexer::plugin_state::PluginStateBag;
use crate::languages::pascal::main_unit::{build_main_unit_state, PascalProjectState};
use crate::languages::LanguagePlugin;
use crate::types::ParsedFile;

fn empty_file(path: &str) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "pascal".to_string(),
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
        content: Some("{%MainUnit castleutils.pas}\nbegin end.".to_string()),
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

/// End-to-end: a `.inc` fragment with a `{%MainUnit}` directive gets its
/// main unit's own name back as a wildcard `ImportEntry` through the plugin
/// trait method, not just through `main_unit`'s internal state.
#[test]
fn extra_wildcard_imports_reads_main_unit_state_from_bag() {
    let fragment = empty_file("src/castleutils_fragment.inc");
    let state_data = build_main_unit_state(std::slice::from_ref(&fragment), std::path::Path::new(""));
    let mut state = PluginStateBag::new();
    state.set(state_data);

    let entries = PascalPlugin.extra_wildcard_imports(&state, &fragment);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].imported_name, "castleutils");
    assert_eq!(entries[0].module_path.as_deref(), Some("castleutils"));
    assert!(entries[0].is_wildcard);
}

/// No `PascalProjectState` in the bag (e.g. a project with no Pascal
/// fragments at all) — the plugin declines cleanly rather than panicking.
#[test]
fn extra_wildcard_imports_missing_project_state_yields_nothing() {
    let state = PluginStateBag::new();
    let fragment = empty_file("src/castleutils_fragment.inc");

    assert!(PascalPlugin.extra_wildcard_imports(&state, &fragment).is_empty());
}

/// `PascalProjectState` present but this specific file has no entry (a
/// `.pas` main unit file, never keyed by `build_main_unit_state`).
#[test]
fn extra_wildcard_imports_file_with_no_entry_yields_nothing() {
    let state_data = PascalProjectState::from_map(std::collections::HashMap::new());
    let mut state = PluginStateBag::new();
    state.set(state_data);
    let file = empty_file("src/castleutils.pas");

    assert!(PascalPlugin.extra_wildcard_imports(&state, &file).is_empty());
}
