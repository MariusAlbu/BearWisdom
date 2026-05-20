use super::*;
use crate::indexer::resolve::engine::{FileContext, RefContext, LanguageResolver};
use crate::types::*;

#[test]
fn test_hare_http_emit() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    let r = ExtractedRef {
        source_symbol_index: 0,
        target_name: "get".to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        col: 0,
        module: Some("net::http::client".to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: vec![CallArg::StringLit("https://api.example.com/x".to_string())],
    };
    let sym = ExtractedSymbol {
        name: "main".to_string(), qualified_name: "main".to_string(),
        kind: SymbolKind::Function, visibility: Some(Visibility::Public),
        start_line: 1, end_line: 1, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let rc = RefContext { extracted_ref: &r, source_symbol: &sym, scope_chain: vec![], file_package_id: None };
    let fc = FileContext { file_path: "x.ha".to_string(), language: "hare".to_string(), imports: vec![], file_namespace: None };
    assert!(matches!(super::detect_flow_inner(&fc, &rc).first(), Some(FlowEmission::NamedChannel { .. })));
}

#[test]
fn test_hare_no_emit_for_non_http_module() {
    let r = ExtractedRef {
        source_symbol_index: 0,
        target_name: "get".to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        col: 0,
        module: Some("io::map".to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: vec![CallArg::StringLit("/x".to_string())],
    };
    let sym = ExtractedSymbol {
        name: "main".to_string(), qualified_name: "main".to_string(),
        kind: SymbolKind::Function, visibility: Some(Visibility::Public),
        start_line: 1, end_line: 1, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let rc = RefContext { extracted_ref: &r, source_symbol: &sym, scope_chain: vec![], file_package_id: None };
    let fc = FileContext { file_path: "x.ha".to_string(), language: "hare".to_string(), imports: vec![], file_namespace: None };
    assert!(super::detect_flow_inner(&fc, &rc).is_empty());
}

#[test]
fn test_hare_no_emit_for_non_url_arg() {
    let r = ExtractedRef {
        source_symbol_index: 0,
        target_name: "get".to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        col: 0,
        module: Some("net::http".to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: vec![CallArg::StringLit("notaurl".to_string())],
    };
    let sym = ExtractedSymbol {
        name: "main".to_string(), qualified_name: "main".to_string(),
        kind: SymbolKind::Function, visibility: Some(Visibility::Public),
        start_line: 1, end_line: 1, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let rc = RefContext { extracted_ref: &r, source_symbol: &sym, scope_chain: vec![], file_package_id: None };
    let fc = FileContext { file_path: "x.ha".to_string(), language: "hare".to_string(), imports: vec![], file_namespace: None };
    assert!(super::detect_flow_inner(&fc, &rc).is_empty());
}
