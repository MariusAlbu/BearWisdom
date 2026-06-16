// =============================================================================
// pascal/resolve_tests.rs — unit tests for pascal/hooks.rs helpers
// =============================================================================

use super::hooks::is_delphi_namespaced_file;
use crate::indexer::resolve::legacy::{FileContext, ImportEntry};

fn wildcard_import(module: &str) -> ImportEntry {
    ImportEntry {
        imported_name: module.to_string(),
        module_path: Some(module.to_string()),
        alias: None,
        is_wildcard: true,
    }
}

fn file_ctx_with_imports(imports: Vec<ImportEntry>) -> FileContext {
    FileContext {
        file_path: "src/main.pas".to_string(),
        language: "pascal".to_string(),
        imports,
        file_namespace: None,
    }
}

// ---------------------------------------------------------------------------
// is_delphi_namespaced_file
// ---------------------------------------------------------------------------

#[test]
fn delphi_file_detected_by_vcl_prefix() {
    let ctx = file_ctx_with_imports(vec![
        wildcard_import("Vcl.Forms"),
        wildcard_import("Vcl.Controls"),
    ]);
    assert!(is_delphi_namespaced_file(&ctx));
}

#[test]
fn delphi_file_detected_by_winapi_prefix() {
    let ctx = file_ctx_with_imports(vec![wildcard_import("Winapi.Windows")]);
    assert!(is_delphi_namespaced_file(&ctx));
}

#[test]
fn delphi_file_detected_by_firedac_prefix() {
    let ctx = file_ctx_with_imports(vec![wildcard_import("FireDAC.Comp.Client")]);
    assert!(is_delphi_namespaced_file(&ctx));
}

#[test]
fn fpc_file_not_classified_as_delphi() {
    // FPC / Lazarus files import with unqualified unit names.
    let ctx = file_ctx_with_imports(vec![
        wildcard_import("SysUtils"),
        wildcard_import("Classes"),
        wildcard_import("LCLType"),
    ]);
    assert!(!is_delphi_namespaced_file(&ctx));
}

#[test]
fn mixed_imports_classified_as_delphi_when_any_prefix_matches() {
    let ctx = file_ctx_with_imports(vec![
        wildcard_import("SysUtils"),
        wildcard_import("Winapi.Messages"),
    ]);
    assert!(is_delphi_namespaced_file(&ctx));
}

// ---------------------------------------------------------------------------
// Pascal flow emission tests
// ---------------------------------------------------------------------------

use super::hooks::{detect_pascal_db_query, detect_pascal_http_producer};

#[test]
fn test_pascal_idhttp_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![crate::types::CallArg::StringLit(
        "https://api.example.com/x".to_string(),
    )];
    match detect_pascal_http_producer("Get", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_pascal_post_emits_post() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    let args = vec![crate::types::CallArg::StringLit("/api/x".to_string())];
    match detect_pascal_http_producer("Post", &args).unwrap() {
        FlowEmission::NamedChannel { method, .. } => assert_eq!(method, Some(HttpMethod::Post)),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_pascal_rejects_non_http_method() {
    let args = vec![crate::types::CallArg::StringLit("/x".to_string())];
    assert!(detect_pascal_http_producer("DoSomething", &args).is_none());
}

#[test]
fn test_pascal_execsql_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let args = vec![crate::types::CallArg::StringLit(
        "SELECT id FROM users".to_string(),
    )];
    match detect_pascal_db_query("Open", &args).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_pascal_db_rejects_non_sql() {
    let args = vec![crate::types::CallArg::StringLit("not sql".to_string())];
    assert!(detect_pascal_db_query("ExecSQL", &args).is_none());
}

#[test]
fn test_pascal_db_insert_op() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let args = vec![crate::types::CallArg::StringLit(
        "INSERT INTO items VALUES (1)".to_string(),
    )];
    match detect_pascal_db_query("ExecSQL", &args).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}
