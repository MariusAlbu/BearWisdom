use super::*;
use crate::indexer::resolve::engine::{FileContext, RefContext};
use crate::types::*;

fn fixture(target: &str, args: Vec<CallArg>) -> (ExtractedRef, ExtractedSymbol, FileContext) {
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        col: 0,
        module: None,
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
    let fc = FileContext { file_path: "x.f90".to_string(), language: "fortran".to_string(), imports: vec![], file_namespace: None };
    (r, sym, fc)
}

#[test]
fn test_fortran_curl_easy_setopt_emits_producer() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    let (r, sym, fc) = fixture("curl_easy_setopt", vec![CallArg::Other, CallArg::Other, CallArg::StringLit("https://api.example.com/x".to_string())]);
    let rc = RefContext { extracted_ref: &r, source_symbol: &sym, scope_chain: vec![], file_package_id: None };
    assert!(matches!(super::hooks::detect_flow_inner(&fc, &rc).first(), Some(FlowEmission::NamedChannel { .. })));
}

#[test]
fn test_fortran_no_emit_for_non_curl_function() {
    let (r, sym, fc) = fixture("PRINT", vec![CallArg::StringLit("hello".to_string())]);
    let rc = RefContext { extracted_ref: &r, source_symbol: &sym, scope_chain: vec![], file_package_id: None };
    assert!(super::hooks::detect_flow_inner(&fc, &rc).is_empty());
}

#[test]
fn test_fortran_no_emit_for_non_url() {
    let (r, sym, fc) = fixture("curl_easy_setopt", vec![CallArg::StringLit("notaurl".to_string())]);
    let rc = RefContext { extracted_ref: &r, source_symbol: &sym, scope_chain: vec![], file_package_id: None };
    assert!(super::hooks::detect_flow_inner(&fc, &rc).is_empty());
}
