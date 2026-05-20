use super::hooks::{detect_gleam_http_producer, detect_gleam_pgo_emission};
use crate::types::*;

#[test]
fn test_gleam_httpc_send_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("https://api.example.com/x".to_string())];
    match detect_gleam_http_producer("gleam/httpc", "send", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(method, Some(HttpMethod::Any));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_gleam_http_rejects_non_http_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_gleam_http_producer("gleam/list", "send", &args).is_none());
}

#[test]
fn test_gleam_http_get_method() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_gleam_http_producer("gleam/httpc", "get", &args).unwrap() {
        FlowEmission::NamedChannel { method, .. } => assert_eq!(method, Some(HttpMethod::Get)),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_gleam_pog_execute_emits_db_other() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_gleam_pgo_emission("pog", "execute", &[]).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Other),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_gleam_pog_rejects_non_db_module() {
    assert!(detect_gleam_pgo_emission("gleam/list", "execute", &[]).is_none());
}

#[test]
fn test_gleam_sqlight_query_emits_db() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    assert!(matches!(
        detect_gleam_pgo_emission("sqlight", "query", &[]).unwrap(),
        FlowEmission::DbQuery { .. }
    ));
}
