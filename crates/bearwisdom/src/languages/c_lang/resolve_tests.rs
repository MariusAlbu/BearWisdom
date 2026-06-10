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
        file_path: "main.c".to_string(),
        language: "c".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    (r, sym, fc)
}

#[test]
fn test_c_pqexec_select_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let (r, sym, fc) = fixture(
        "PQexec",
        vec![
            CallArg::Other,
            CallArg::StringLit("SELECT id FROM users".to_string()),
        ],
    );
    let rc = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let em = super::hooks::detect_flow_inner(&fc, &rc);
    match em.first().unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(*operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_c_sqlite3_exec_insert_emits_db_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let (r, sym, fc) = fixture(
        "sqlite3_exec",
        vec![
            CallArg::Other,
            CallArg::StringLit("INSERT INTO items VALUES (1)".to_string()),
        ],
    );
    let rc = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    match super::hooks::detect_flow_inner(&fc, &rc).first().unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(*operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_c_no_emit_for_non_db_function() {
    let (r, sym, fc) = fixture("printf", vec![CallArg::StringLit("hello".to_string())]);
    let rc = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    assert!(super::hooks::detect_flow_inner(&fc, &rc).is_empty());
}

#[test]
fn test_c_mysql_query_emits_db() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    let (r, sym, fc) = fixture(
        "mysql_query",
        vec![
            CallArg::Other,
            CallArg::StringLit("UPDATE accounts SET balance = 1".to_string()),
        ],
    );
    let rc = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    assert!(matches!(
        super::hooks::detect_flow_inner(&fc, &rc).first(),
        Some(FlowEmission::DbQuery { .. })
    ));
}
