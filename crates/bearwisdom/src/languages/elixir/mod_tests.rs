use std::collections::HashMap;

use super::ElixirPlugin;
use crate::indexer::plugin_state::PluginStateBag;
use crate::languages::elixir::using_injection::{ElixirInjection, ElixirProjectState};
use crate::languages::LanguagePlugin;
use crate::types::{EdgeKind, ExtractedRef, ParsedFile};

fn use_ref(target_name: &str, module: &str) -> ExtractedRef {
    ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target_name.to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        col: 0,
        module: Some(module.to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn alias_ref(target_name: &str, module: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: true,
        ..use_ref(target_name, module)
    }
}

fn file_with_refs(refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: "lib/plausible_web/data_case_user.ex".to_string(),
        language: "elixir".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

fn state_with_injections(injections: HashMap<String, Vec<ElixirInjection>>) -> PluginStateBag {
    let mut bag = PluginStateBag::new();
    bag.set(ElixirProjectState::from_map(injections));
    bag
}

/// `use Plausible.DataCase` where `DataCase`'s `__using__` block does
/// `import Plausible.Factory` — the file never mentions `Factory` itself,
/// so a bare `build(:site)` call only resolves once the one-hop redirect
/// turns that injection into a wildcard `ImportEntry`.
#[test]
fn use_of_using_module_yields_wildcard_import_for_its_injected_import() {
    let mut injections = HashMap::new();
    injections.insert(
        "Plausible.DataCase".to_string(),
        vec![ElixirInjection::Import {
            module: "Plausible.Factory".to_string(),
        }],
    );
    let state = state_with_injections(injections);
    let file = file_with_refs(vec![use_ref("DataCase", "Plausible.DataCase")]);

    let entries = ElixirPlugin.extra_wildcard_imports(&state, &file);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].imported_name, "Factory");
    assert_eq!(entries[0].module_path.as_deref(), Some("Plausible.Factory"));
    assert_eq!(entries[0].alias, None);
    assert!(entries[0].is_wildcard);
}

/// `use M`'s injected `alias N.N, as: Local` becomes a non-wildcard entry
/// keyed on the LOCAL bound name (mirroring how Elixir's own directive
/// extraction bakes an `as:` rename into `imported_name` rather than a
/// separate `alias` field), so `AliasModuleQnameRule` binds it.
#[test]
fn use_of_using_module_yields_aliased_import_for_its_injected_alias() {
    let mut injections = HashMap::new();
    injections.insert(
        "Plausible.ConnCase".to_string(),
        vec![ElixirInjection::Alias {
            local: "Router".to_string(),
            module: "PlausibleWeb.Router".to_string(),
        }],
    );
    let state = state_with_injections(injections);
    let file = file_with_refs(vec![use_ref("ConnCase", "Plausible.ConnCase")]);

    let entries = ElixirPlugin.extra_wildcard_imports(&state, &file);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].imported_name, "Router");
    assert_eq!(
        entries[0].module_path.as_deref(),
        Some("PlausibleWeb.Router")
    );
    assert_eq!(entries[0].alias, None);
    assert!(!entries[0].is_wildcard);
}

/// A nested `use N` inside M's own quote block whose target has no recorded
/// injections of its own (never defines `__using__`, or an empty quote)
/// terminates the hop with nothing — it must not panic or fabricate an entry.
#[test]
fn nested_use_injection_with_no_downstream_data_produces_no_entry() {
    let mut injections = HashMap::new();
    injections.insert(
        "Plausible.DataCase".to_string(),
        vec![ElixirInjection::Use {
            module: "Plausible.TestUtils".to_string(),
        }],
    );
    let state = state_with_injections(injections);
    let file = file_with_refs(vec![use_ref("DataCase", "Plausible.DataCase")]);

    assert!(ElixirPlugin.extra_wildcard_imports(&state, &file).is_empty());
}

/// A nested `use N` inside M's own quote block, where N itself has a
/// recorded injection set, is followed transitively — `use M` sees N's
/// injected `import` too, not just M's own.
#[test]
fn nested_use_injection_is_expanded_transitively() {
    let mut injections = HashMap::new();
    injections.insert(
        "Plausible.DataCase".to_string(),
        vec![ElixirInjection::Use {
            module: "Plausible.TestUtils".to_string(),
        }],
    );
    injections.insert(
        "Plausible.TestUtils".to_string(),
        vec![ElixirInjection::Import {
            module: "Plausible.Factory".to_string(),
        }],
    );
    let state = state_with_injections(injections);
    let file = file_with_refs(vec![use_ref("DataCase", "Plausible.DataCase")]);

    let entries = ElixirPlugin.extra_wildcard_imports(&state, &file);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].imported_name, "Factory");
    assert_eq!(entries[0].module_path.as_deref(), Some("Plausible.Factory"));
    assert!(entries[0].is_wildcard);
}

/// `alias Plausible.DataCase` (a binding directive, `is_import_binding =
/// true`) never invokes `__using__` — it must not pick up DataCase's
/// injections even though the module itself is `__using__`-defining.
#[test]
fn alias_directive_is_not_treated_as_a_use_site() {
    let mut injections = HashMap::new();
    injections.insert(
        "Plausible.DataCase".to_string(),
        vec![ElixirInjection::Import {
            module: "Plausible.Factory".to_string(),
        }],
    );
    let state = state_with_injections(injections);
    let file = file_with_refs(vec![alias_ref("DataCase", "Plausible.DataCase")]);

    assert!(ElixirPlugin.extra_wildcard_imports(&state, &file).is_empty());
}

/// A `use` of a module with no recorded injections (never defines
/// `__using__`, or defines one with an empty quote block) contributes
/// nothing — the common case for most `use` directives in a real project.
#[test]
fn use_of_module_with_no_injections_yields_nothing() {
    let state = state_with_injections(HashMap::new());
    let file = file_with_refs(vec![use_ref("Enum", "Enum")]);

    assert!(ElixirPlugin.extra_wildcard_imports(&state, &file).is_empty());
}

/// No `ElixirProjectState` stored in the bag at all (e.g. a project with no
/// Elixir files) — the plugin must decline cleanly rather than panic.
#[test]
fn missing_project_state_yields_nothing() {
    let state = PluginStateBag::new();
    let file = file_with_refs(vec![use_ref("DataCase", "Plausible.DataCase")]);

    assert!(ElixirPlugin.extra_wildcard_imports(&state, &file).is_empty());
}

fn module_symbol(qualified_name: &str) -> crate::types::ExtractedSymbol {
    crate::types::ExtractedSymbol {
        name: qualified_name.rsplit('.').next().unwrap_or(qualified_name).to_string(),
        qualified_name: qualified_name.to_string(),
        kind: crate::types::SymbolKind::Module,
        visibility: Some(crate::types::Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn file_with_symbols(path: &str, symbols: Vec<crate::types::ExtractedSymbol>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        symbols,
        ..file_with_refs(Vec::new())
    }
}

/// `synthesize_project_symbols` delegates to the harvested `Def` facts:
/// `Plausible.Factory`'s `use ExMachina.Ecto`-shaped chain is stored in the
/// bag as a direct `Def` entry here (the harvest itself is covered in
/// `using_synthesis_tests.rs`) — this test only proves the plugin wiring
/// reads the bag and returns what the synthesis module computes.
#[test]
fn synthesize_project_symbols_returns_harvested_def_as_new_member() {
    let mut injections = HashMap::new();
    injections.insert(
        "Plausible.Factory".to_string(),
        vec![ElixirInjection::Def {
            name: "build".to_string(),
            is_macro: false,
        }],
    );
    let state = state_with_injections(injections);
    let file = file_with_symbols(
        "test/support/factory.ex",
        vec![module_symbol("Plausible.Factory")],
    );

    let out = ElixirPlugin.synthesize_project_symbols(&state, &[file]);

    assert_eq!(out.len(), 1);
    let (path, syms) = &out[0];
    assert_eq!(path, "test/support/factory.ex");
    assert_eq!(syms.len(), 1);
    assert_eq!(syms[0].qualified_name, "Plausible.Factory.build");
}

/// No `ElixirProjectState` in the bag — decline cleanly, same contract as
/// `extra_wildcard_imports`.
#[test]
fn synthesize_project_symbols_missing_project_state_yields_nothing() {
    let state = PluginStateBag::new();
    let file = file_with_symbols(
        "test/support/factory.ex",
        vec![module_symbol("Plausible.Factory")],
    );

    assert!(ElixirPlugin
        .synthesize_project_symbols(&state, &[file])
        .is_empty());
}
