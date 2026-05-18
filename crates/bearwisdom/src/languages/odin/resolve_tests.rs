use super::*;
use crate::indexer::resolve::engine::{FileContext, RefContext, LanguageResolver};
use crate::types::*;

fn fixture(target: &str, module: &str, args: Vec<CallArg>) -> (ExtractedRef, ExtractedSymbol, FileContext) {
    let r = ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        col: 0,
        module: Some(module.to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: args,
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
    let fc = FileContext { file_path: "main.odin".to_string(), language: "odin".to_string(), imports: vec![], file_namespace: None };
    (r, sym, fc)
}

#[test]
fn test_odin_http_emit() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    let (r, sym, fc) = fixture("get", "vendor:http", vec![CallArg::StringLit("https://api.example.com/x".to_string())]);
    let rc = RefContext { extracted_ref: &r, source_symbol: &sym, scope_chain: vec![], file_package_id: None };
    assert!(matches!(OdinResolver.detect_flow_emission(&fc, &rc).first(), Some(FlowEmission::NamedChannel { .. })));
}

#[test]
fn test_odin_no_emit_for_non_http_module() {
    let (r, sym, fc) = fixture("get", "core:fmt", vec![CallArg::StringLit("/x".to_string())]);
    let rc = RefContext { extracted_ref: &r, source_symbol: &sym, scope_chain: vec![], file_package_id: None };
    assert!(OdinResolver.detect_flow_emission(&fc, &rc).is_empty());
}

#[test]
fn test_odin_no_emit_for_non_url() {
    let (r, sym, fc) = fixture("get", "vendor:http", vec![CallArg::StringLit("notaurl".to_string())]);
    let rc = RefContext { extracted_ref: &r, source_symbol: &sym, scope_chain: vec![], file_package_id: None };
    assert!(OdinResolver.detect_flow_emission(&fc, &rc).is_empty());
}
