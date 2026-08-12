use std::collections::HashMap;

use super::*;
use crate::types::{FlowMeta, Visibility};

fn module_symbol(qualified_name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: qualified_name.rsplit('.').next().unwrap_or(qualified_name).to_string(),
        qualified_name: qualified_name.to_string(),
        kind: SymbolKind::Module,
        visibility: Some(Visibility::Public),
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

fn method_symbol(qualified_name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        kind: SymbolKind::Method,
        ..module_symbol(qualified_name)
    }
}

fn ex_file(path: &str, symbols: Vec<ExtractedSymbol>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "elixir".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
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
    }
}

fn state_with(injections: HashMap<String, Vec<ElixirInjection>>) -> ElixirProjectState {
    ElixirProjectState::from_map(injections)
}

#[test]
fn synthesizes_def_member_for_module_with_flattened_def() {
    let mut injections = HashMap::new();
    injections.insert(
        "Plausible.Factory".to_string(),
        vec![ElixirInjection::Def {
            name: "build".to_string(),
            is_macro: false,
        }],
    );
    let state = state_with(injections);
    let pf = ex_file(
        "test/support/factory.ex",
        vec![module_symbol("Plausible.Factory")],
    );

    let out = synthesize_def_members(&state, &[pf]);

    assert_eq!(out.len(), 1);
    let (path, syms) = &out[0];
    assert_eq!(path, "test/support/factory.ex");
    assert_eq!(syms.len(), 1);
    assert_eq!(syms[0].qualified_name, "Plausible.Factory.build");
    assert_eq!(syms[0].kind, SymbolKind::Method);
    assert_eq!(syms[0].parent_index, None);
}

#[test]
fn macro_def_synthesizes_as_function_kind() {
    let mut injections = HashMap::new();
    injections.insert(
        "MyApp.Case".to_string(),
        vec![ElixirInjection::Def {
            name: "test".to_string(),
            is_macro: true,
        }],
    );
    let state = state_with(injections);
    let pf = ex_file("lib/case.ex", vec![module_symbol("MyApp.Case")]);

    let out = synthesize_def_members(&state, &[pf]);

    assert_eq!(out[0].1[0].kind, SymbolKind::Function);
}

#[test]
fn existing_real_def_is_not_duplicated() {
    // Factory hand-writes its own `build/1` (overriding the macro-injected
    // one) — the synthetic entry must not shadow or duplicate it.
    let mut injections = HashMap::new();
    injections.insert(
        "Plausible.Factory".to_string(),
        vec![ElixirInjection::Def {
            name: "build".to_string(),
            is_macro: false,
        }],
    );
    let state = state_with(injections);
    let pf = ex_file(
        "test/support/factory.ex",
        vec![
            module_symbol("Plausible.Factory"),
            method_symbol("Plausible.Factory.build"),
        ],
    );

    let out = synthesize_def_members(&state, &[pf]);

    assert!(out.is_empty());
}

#[test]
fn module_with_no_flattened_defs_yields_nothing() {
    let mut injections = HashMap::new();
    injections.insert(
        "Plausible.DataCase".to_string(),
        vec![ElixirInjection::Import {
            module: "Plausible.Factory".to_string(),
        }],
    );
    let state = state_with(injections);
    let pf = ex_file("test/support/data_case.ex", vec![module_symbol("Plausible.DataCase")]);

    let out = synthesize_def_members(&state, &[pf]);

    assert!(out.is_empty());
}

#[test]
fn duplicate_def_reached_through_two_hops_is_synthesized_once() {
    // Two `use` chains both terminate on a module defining `build` — the
    // flattened set carries the fact twice; the synthesized output must not.
    let mut injections = HashMap::new();
    injections.insert(
        "MyApp.Factory".to_string(),
        vec![
            ElixirInjection::Use {
                module: "MyApp.A".to_string(),
            },
            ElixirInjection::Use {
                module: "MyApp.B".to_string(),
            },
        ],
    );
    injections.insert(
        "MyApp.A".to_string(),
        vec![ElixirInjection::Def {
            name: "build".to_string(),
            is_macro: false,
        }],
    );
    injections.insert(
        "MyApp.B".to_string(),
        vec![ElixirInjection::Def {
            name: "build".to_string(),
            is_macro: false,
        }],
    );
    let state = state_with(injections);
    let pf = ex_file("lib/factory.ex", vec![module_symbol("MyApp.Factory")]);

    let out = synthesize_def_members(&state, &[pf]);

    assert_eq!(out[0].1.len(), 1);
}

#[test]
fn non_elixir_file_is_skipped() {
    let mut injections = HashMap::new();
    injections.insert(
        "Plausible.Factory".to_string(),
        vec![ElixirInjection::Def {
            name: "build".to_string(),
            is_macro: false,
        }],
    );
    let state = state_with(injections);
    let mut pf = ex_file(
        "test/support/factory.ex",
        vec![module_symbol("Plausible.Factory")],
    );
    pf.language = "ruby".to_string();

    let out = synthesize_def_members(&state, &[pf]);

    assert!(out.is_empty());
}
