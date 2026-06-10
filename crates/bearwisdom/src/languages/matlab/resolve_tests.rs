use super::*;
use crate::indexer::resolve::engine::{FileContext, RefContext};
use crate::types::*;

fn fixture(target: &str, args: Vec<CallArg>) -> (ExtractedRef, ExtractedSymbol, FileContext) {
    let r = ExtractedRef {
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
        call_args: args,
    };
    let sym = ExtractedSymbol {
        name: "main".to_string(),
        qualified_name: "main".to_string(),
        kind: SymbolKind::Function,
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
    };
    let fc = FileContext {
        file_path: "x.m".to_string(),
        language: "matlab".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    (r, sym, fc)
}

#[test]
fn test_matlab_webread_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    let (r, sym, fc) = fixture(
        "webread",
        vec![CallArg::StringLit("https://api.example.com/x".to_string())],
    );
    let rc = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    match super::hooks::detect_flow_inner(&fc, &rc).first().unwrap() {
        FlowEmission::NamedChannel { role, .. } => assert_eq!(*role, ChannelRole::Producer),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_matlab_fetch_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let (r, sym, fc) = fixture(
        "fetch",
        vec![
            CallArg::Other,
            CallArg::StringLit("SELECT * FROM users".to_string()),
        ],
    );
    let rc = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    match super::hooks::detect_flow_inner(&fc, &rc).first().unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(*operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_matlab_no_emit_for_non_url() {
    let (r, sym, fc) = fixture("webread", vec![CallArg::StringLit("notaurl".to_string())]);
    let rc = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    assert!(super::hooks::detect_flow_inner(&fc, &rc).is_empty());
}
