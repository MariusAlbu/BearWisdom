use super::hooks::{detect_perl_dbi_emission, detect_perl_http_producer, detect_perl_route};
use crate::types::*;

#[test]
fn test_perl_dancer_get_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_perl_route("get", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_perl_dancer_post_works() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_perl_route("post", &args).is_some());
}

#[test]
fn test_perl_route_rejects_unknown() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_perl_route("hook", &args).is_none());
}

#[test]
fn test_perl_lwp_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    let args = vec![CallArg::StringLit("https://api.example.com/x".to_string())];
    match detect_perl_http_producer("LWP::UserAgent", "get", &args).unwrap() {
        FlowEmission::NamedChannel { role, .. } => assert_eq!(role, ChannelRole::Producer),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_perl_http_rejects_non_http_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_perl_http_producer("Data::Dumper", "get", &args).is_none());
}

#[test]
fn test_perl_dbi_select_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let args = vec![CallArg::StringLit(
        "SELECT id, name FROM users WHERE active = 1".to_string(),
    )];
    match detect_perl_dbi_emission("DBI", "prepare", &args).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_perl_dbi_insert_emits_db_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let args = vec![CallArg::StringLit(
        "INSERT INTO items (a) VALUES (1)".to_string(),
    )];
    match detect_perl_dbi_emission("DBI", "do", &args).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}
