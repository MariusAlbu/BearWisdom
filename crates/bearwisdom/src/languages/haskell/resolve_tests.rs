use super::hooks::{detect_haskell_http_producer, detect_haskell_persistent_emission, detect_haskell_scotty_route};
use crate::types::*;

#[test]
fn test_haskell_scotty_get_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_haskell_scotty_route("get", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_haskell_scotty_post_works() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_haskell_scotty_route("post", &args).is_some());
}

#[test]
fn test_haskell_scotty_rejects_unknown() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_haskell_scotty_route("middleware", &args).is_none());
}

#[test]
fn test_haskell_http_producer_emits() {
    use crate::indexer::resolve::flow_emit::ChannelRole;
    let args = vec![CallArg::StringLit("https://api.example.com/x".to_string())];
    if let crate::indexer::resolve::flow_emit::FlowEmission::NamedChannel { role, .. } =
        detect_haskell_http_producer("Network.Wreq", "get", &args).unwrap()
    {
        assert_eq!(role, ChannelRole::Producer);
    } else {
        panic!("expected NamedChannel");
    }
}

#[test]
fn test_haskell_http_rejects_non_http_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_haskell_http_producer("Data.List", "get", &args).is_none());
}

#[test]
fn test_haskell_persistent_select_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_haskell_persistent_emission("Database.Persist.Sql", "selectList").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_haskell_persistent_insert_emits_db_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_haskell_persistent_emission("Database.Persist", "insert").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}
