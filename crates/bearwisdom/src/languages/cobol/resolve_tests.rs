use super::*;
use crate::indexer::resolve::engine::{FileContext, RefContext, LanguageResolver};
use crate::types::*;

fn fixture(target: &str, args: Vec<CallArg>) -> (ExtractedRef, ExtractedSymbol, FileContext) {
    let r = ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 1,
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
    };
    let fc = FileContext { file_path: "x.cbl".to_string(), language: "cobol".to_string(), imports: vec![], file_namespace: None };
    (r, sym, fc)
}

#[test]
fn test_cobol_exec_sql_select_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let (r, sym, fc) = fixture("EXEC_SQL", vec![CallArg::StringLit("SELECT * FROM customers".to_string())]);
    let rc = RefContext { extracted_ref: &r, source_symbol: &sym, scope_chain: vec![], file_package_id: None };
    match CobolResolver.detect_flow_emission(&fc, &rc).first().unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(*operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_cobol_exec_sql_insert_emits_db_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let (r, sym, fc) = fixture("EXEC_SQL", vec![CallArg::StringLit("INSERT INTO orders VALUES (1)".to_string())]);
    let rc = RefContext { extracted_ref: &r, source_symbol: &sym, scope_chain: vec![], file_package_id: None };
    match CobolResolver.detect_flow_emission(&fc, &rc).first().unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(*operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_cobol_no_emit_for_non_sql() {
    let (r, sym, fc) = fixture("DISPLAY", vec![CallArg::StringLit("hello".to_string())]);
    let rc = RefContext { extracted_ref: &r, source_symbol: &sym, scope_chain: vec![], file_package_id: None };
    assert!(CobolResolver.detect_flow_emission(&fc, &rc).is_empty());
}

#[test]
fn test_cobol_update_op() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let (r, sym, fc) = fixture("EXEC_SQL", vec![CallArg::StringLit("UPDATE accounts SET balance = 0".to_string())]);
    let rc = RefContext { extracted_ref: &r, source_symbol: &sym, scope_chain: vec![], file_package_id: None };
    match CobolResolver.detect_flow_emission(&fc, &rc).first().unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(*operation, DbQueryOp::Update),
        _ => panic!("expected DbQuery"),
    }
}
