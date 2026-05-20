use super::hooks::{detect_fsharp_db_query, detect_fsharp_http_producer, detect_fsharp_route};
use crate::types::*;

#[test]
fn test_fsharp_route_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_fsharp_route("route", &args).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "/api/users");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_fsharp_saturn_get_emits_get_method() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/x".to_string())];
    match detect_fsharp_route("GET", &args).unwrap() {
        FlowEmission::NamedChannel { method, .. } => assert_eq!(method, Some(HttpMethod::Get)),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_fsharp_route_rejects_unknown() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_fsharp_route("middleware", &args).is_none());
}

#[test]
fn test_fsharp_http_producer_emits() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    let args = vec![CallArg::StringLit("https://api.example.com/x".to_string())];
    assert!(matches!(
        detect_fsharp_http_producer("FSharp.Data.Http", "AsyncRequestString", &args).unwrap(),
        FlowEmission::NamedChannel { .. }
    ));
}

#[test]
fn test_fsharp_http_rejects_non_http_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_fsharp_http_producer("List", "AsyncRequestString", &args).is_none());
}

#[test]
fn test_fsharp_db_query_executes() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_fsharp_db_query("Npgsql.FSharp.Sql", "executeAsync").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Other),
        _ => panic!("expected DbQuery"),
    }
}
