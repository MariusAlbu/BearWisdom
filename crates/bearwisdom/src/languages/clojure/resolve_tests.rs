use super::hooks::{
    detect_clj_compojure_route, detect_clj_http_producer, detect_clj_jdbc_db_query,
};
use crate::types::*;

#[test]
fn test_clj_compojure_get_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_clj_compojure_route("GET", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_clj_compojure_post_works() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_clj_compojure_route("POST", &args).is_some());
}

#[test]
fn test_clj_compojure_rejects_lowercase() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_clj_compojure_route("get", &args).is_none());
}

#[test]
fn test_clj_http_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_clj_http_producer("clj-http.client", "get", &args).unwrap() {
        FlowEmission::NamedChannel { role, .. } => assert_eq!(role, ChannelRole::Producer),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_clj_http_rejects_non_http_ns() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_clj_http_producer("clojure.core", "get", &args).is_none());
}

#[test]
fn test_clj_jdbc_execute_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_clj_jdbc_db_query("next.jdbc", "execute!").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_clj_jdbc_insert_emits_db_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_clj_jdbc_db_query("next.jdbc.sql", "insert!").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}
