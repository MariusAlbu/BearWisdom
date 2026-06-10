// Tests for bicep external classification — Azure resource types and child
// resource shorthand routed via `classify_external`.

use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolIndex};
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility,
};
use std::collections::HashMap;

fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "bicep".to_string(),
        content_hash: "x".to_string(),
        size: 100,
        line_count: 10,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    }
}

fn make_sym(name: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 1,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn make_calls(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_type_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::TypeRef,
        line: 1,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

#[test]
fn child_resource_shorthand_classifies_as_azure() {
    let sym = make_sym("azureFirewallSubnet", SymbolKind::Class);
    for name in ["subnets", "ruleCollectionGroups", "virtualNetworkLinks"] {
        let tr = make_type_ref(name);
        let file = make_file("n.bicep", vec![sym.clone()], vec![tr.clone()]);
        let parsed = vec![file];
        let index = SymbolIndex::build(&parsed, &HashMap::new());
        let file_ctx = {
            use crate::type_checker::profile::hooks::LanguageEngineHooks;
            super::hooks::BicepHooks
                .build_file_context(&parsed[0], None)
                .unwrap()
        };
        let ref_ctx = RefContext {
            extracted_ref: &tr,
            source_symbol: &sym,
            scope_chain: vec![],
            file_package_id: None,
        };
        let ns = {
            use crate::type_checker::profile::hooks::LanguageEngineHooks;
            crate::languages::bicep::hooks::BicepHooks
                .classify_external(&ref_ctx, &file_ctx, None, &index)
        };
        assert_eq!(
            ns.as_deref(),
            Some("azure"),
            "child-resource shorthand `{name}` should classify as azure"
        );
    }
}

#[test]
fn user_symbol_not_child_shorthand() {
    // PascalCase names are user symbols, not Azure shortcuts.
    let sym = make_sym("src", SymbolKind::Class);
    let tr = make_type_ref("MyOwnResource");
    let file = make_file("n.bicep", vec![sym.clone()], vec![tr.clone()]);
    let parsed = vec![file];
    let index = SymbolIndex::build(&parsed, &HashMap::new());
    let file_ctx = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        super::hooks::BicepHooks
            .build_file_context(&parsed[0], None)
            .unwrap()
    };
    let ref_ctx = RefContext {
        extracted_ref: &tr,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        crate::languages::bicep::hooks::BicepHooks
            .classify_external(&ref_ctx, &file_ctx, None, &index)
    };
    assert_eq!(
        ns, None,
        "PascalCase target should not be routed as azure shorthand"
    );
}
