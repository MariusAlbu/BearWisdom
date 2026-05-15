use super::*;
use crate::types::*;

#[test]
fn test_erlang_httpc_request_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    let args = vec![CallArg::Ident("get".to_string()), CallArg::StringLit("https://api.example.com/x".to_string())];
    match detect_erlang_http_emission("httpc", "request", &args).unwrap() {
        FlowEmission::NamedChannel { kind, role, method, .. } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(method, Some(HttpMethod::Any));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_erlang_hackney_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_erlang_http_emission("hackney", "get", &args).unwrap() {
        FlowEmission::NamedChannel { method, .. } => assert_eq!(method, Some(HttpMethod::Get)),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_erlang_http_rejects_non_http_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_erlang_http_emission("lists", "get", &args).is_none());
}

#[test]
fn test_erlang_mnesia_read_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_erlang_db_emission("mnesia", "read", &[]).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_erlang_mnesia_write_emits_db_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_erlang_db_emission("mnesia", "write", &[]).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_erlang_db_rejects_non_db() {
    assert!(detect_erlang_db_emission("io", "format", &[]).is_none());
}
