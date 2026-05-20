use super::hooks::{detect_nim_db_emission, detect_nim_http_producer, detect_nim_jester_route};
use crate::types::*;

#[test]
fn test_nim_jester_get_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_nim_jester_route("get", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_nim_jester_post_works() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_nim_jester_route("post", &args).is_some());
}

#[test]
fn test_nim_route_rejects_unknown() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_nim_jester_route("middleware", &args).is_none());
}

#[test]
fn test_nim_httpclient_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    let args = vec![CallArg::StringLit("https://api.example.com/x".to_string())];
    match detect_nim_http_producer("std/httpclient", "getContent", &args).unwrap() {
        FlowEmission::NamedChannel { role, .. } => assert_eq!(role, ChannelRole::Producer),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_nim_http_rejects_non_http_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_nim_http_producer("strutils", "get", &args).is_none());
}

#[test]
fn test_nim_db_exec_emits_db_other() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_nim_db_emission("db_postgres", "exec", &[]).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Other),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_nim_db_get_row_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_nim_db_emission("db_sqlite", "getRow", &[]).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}
