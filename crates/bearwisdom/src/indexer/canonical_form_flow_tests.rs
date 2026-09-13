// Tests for canonical_form_flow.rs — FILE-004 bounds on the flow maps.

use super::check_flow_meta;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility,
};

fn one_symbol_one_ref() -> ParsedFile {
    ParsedFile {
        path: "src/file.php".to_string(),
        language: "php".to_string(),
        content_hash: "h".to_string(),
        size: 1024,
        line_count: 10,
        mtime: None,
        package_id: None,
        symbols: vec![ExtractedSymbol {
            name: "factory".to_string(),
            qualified_name: "Host.factory".to_string(),
            kind: SymbolKind::Property,
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
        }],
        refs: vec![ExtractedRef {
            source_symbol_index: 0,
            target_name: "Factory".to_string(),
            kind: EdgeKind::Instantiates,
            line: 4,
            col: 0,
            module: None,
            namespace_segments: Vec::new(),
            chain: None,
            byte_offset: 32,
            call_args: Vec::new(),
            is_import_binding: false,
            is_reexport: false,
            is_include: false,
        }],
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

fn violations(file: &ParsedFile) -> Vec<&'static str> {
    let mut out = Vec::new();
    check_flow_meta(file, &mut out);
    out.into_iter().map(|v| v.code).collect()
}

#[test]
fn an_in_bounds_member_initializer_is_clean() {
    let mut file = one_symbol_one_ref();
    file.flow.flow_member_init.insert(0, 0);
    assert!(violations(&file).is_empty());
}

#[test]
fn an_out_of_bounds_member_initializer_is_a_contract_violation() {
    let mut file = one_symbol_one_ref();
    file.flow.flow_member_init.insert(99, 0);
    assert_eq!(violations(&file), vec!["FILE-004"]);

    let mut file = one_symbol_one_ref();
    file.flow.flow_member_init.insert(0, 99);
    assert_eq!(violations(&file), vec!["FILE-004"]);
}
