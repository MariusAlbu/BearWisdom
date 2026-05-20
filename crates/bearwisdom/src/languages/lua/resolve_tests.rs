use super::hooks::{detect_lua_db_emission, detect_lua_lapis_route, detect_lua_resty_http};
use crate::types::*;

#[test]
fn test_lua_lapis_get_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_lua_lapis_route("", "get", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_lua_lapis_post_works() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_lua_lapis_route("", "post", &args).is_some());
}

#[test]
fn test_lua_lapis_rejects_unknown() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_lua_lapis_route("", "middleware", &args).is_none());
}

#[test]
fn test_lua_resty_http_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    let args = vec![CallArg::StringLit("https://api.example.com/x".to_string())];
    match detect_lua_resty_http("resty.http", "request_uri", &args).unwrap() {
        FlowEmission::NamedChannel { role, .. } => assert_eq!(role, ChannelRole::Producer),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_lua_http_rejects_non_http_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_lua_resty_http("string", "request", &args).is_none());
}

#[test]
fn test_lua_pgmoon_query_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_lua_db_emission("pgmoon", "query").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_lua_db_rejects_non_db_module() {
    assert!(detect_lua_db_emission("io", "query").is_none());
}
