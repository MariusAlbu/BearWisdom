// Tests for bicep/resolve.rs — decorator-builtin filtering and module path resolution.

use super::resolve::BicepResolver;
use crate::indexer::resolve::engine::{
    FileContext, LanguageResolver, RefContext, SymbolIndex,
};
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
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
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
    }
}

fn make_calls(target: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    }
}

fn make_type_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::TypeRef,
        line: 1,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    }
}

#[test]
fn take_and_pickzones_classify_as_builtin() {
    let sym = make_sym("vnet", SymbolKind::Class);
    for name in ["take", "pickZones"] {
        let calls = make_calls(name);
        let file = make_file("n.bicep", vec![sym.clone()], vec![calls.clone()]);
        let parsed = vec![file];
        let index = SymbolIndex::build(&parsed, &HashMap::new());
        let resolver = BicepResolver;
        let file_ctx = resolver.build_file_context(&parsed[0], None);
        let ref_ctx = RefContext {
            extracted_ref: &calls,
            source_symbol: &sym,
            scope_chain: vec![],
            file_package_id: None,
        };
        let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
        assert_eq!(ns.as_deref(), Some("builtin"), "{name} should be bicep builtin");
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
        let resolver = BicepResolver;
        let file_ctx = resolver.build_file_context(&parsed[0], None);
        let ref_ctx = RefContext {
            extracted_ref: &tr,
            source_symbol: &sym,
            scope_chain: vec![],
            file_package_id: None,
        };
        let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
        assert_eq!(ns.as_deref(), Some("azure"),
            "child-resource shorthand `{name}` should classify as azure");
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
    let resolver = BicepResolver;
    let file_ctx = resolver.build_file_context(&parsed[0], None);
    let ref_ctx = RefContext {
        extracted_ref: &tr,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns, None, "PascalCase target should not be routed as azure shorthand");
}

#[test]
fn decorator_description_classifies_as_builtin() {
    let param = make_sym("location", SymbolKind::Variable);
    let calls = make_calls("description");
    let file = make_file("hub.bicep", vec![param.clone()], vec![calls.clone()]);
    let parsed = vec![file];
    let id_map: HashMap<(String, String), i64> = HashMap::new();
    let index = SymbolIndex::build(&parsed, &id_map);

    let resolver = BicepResolver;
    let file_ctx = resolver.build_file_context(&parsed[0], None);
    let ref_ctx = RefContext {
        extracted_ref: &calls,
        source_symbol: &param,
        scope_chain: vec![],
        file_package_id: None,
    };

    assert!(
        resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "description should not resolve to a project symbol"
    );
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns.as_deref(), Some("builtin"),
        "decorator `description` should be classified as bicep builtin; got {:?}", ns);
}
