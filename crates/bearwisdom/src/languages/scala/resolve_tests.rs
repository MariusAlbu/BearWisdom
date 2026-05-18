use super::*;
use crate::types::*;

fn make_chain(segments: &[&str]) -> MemberChain {
    MemberChain {
        segments: segments
            .iter()
            .enumerate()
            .map(|(i, name)| ChainSegment {
                name: name.to_string(),
                node_kind: "test".to_string(),
                kind: if i == 0 { SegmentKind::Identifier } else { SegmentKind::Property },
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                type_arg_ids: Vec::new(),
})
            .collect(),
    }
}

#[test]
fn test_scala_http_path_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let args = vec![CallArg::StringLit("users".to_string())];
    match detect_scala_http_path_call("path", &args).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "/users");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_scala_path_prefix_works() {
    let args = vec![CallArg::StringLit("api".to_string())];
    assert!(detect_scala_http_path_call("pathPrefix", &args).is_some());
}

#[test]
fn test_scala_http_path_rejects_unrelated() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_scala_http_path_call("other", &args).is_none());
}

#[test]
fn test_scala_client_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    let chain = make_chain(&["client", "get"]);
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_scala_http_chain_emission(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { kind, role, method, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(method, Some(HttpMethod::Get));
            assert_eq!(name, "/api/users");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_scala_slick_result_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["Users", "filter", "result"]);
    match detect_scala_db_query_emission(&chain, &[]).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "scala.Users");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_scala_slick_insert_emits_db_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["Users", "insert"]);
    match detect_scala_db_query_emission(&chain, &[]).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_scala_db_rejects_lowercase_root() {
    let chain = make_chain(&["users", "result"]);
    assert!(detect_scala_db_query_emission(&chain, &[]).is_none());
}

#[test]
fn test_scala_grpc_emits_rpc_call() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let chain = make_chain(&["UserServiceGrpc", "stub", "getUser"]);
    match detect_scala_grpc_emission(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::RpcCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "UserService.getUser");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_scala_grpc_rejects_non_client_root() {
    let chain = make_chain(&["UserService", "stub", "getUser"]);
    assert!(detect_scala_grpc_emission(&chain).is_none());
}

#[test]
fn test_scala_doobie_sql_query_emits_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["sql", "query"]);
    match detect_scala_doobie_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "scala.doobie");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_scala_doobie_sql_update_run_emits_update() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["sql", "update", "run"]);
    match detect_scala_doobie_emission(&chain).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Update),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_scala_doobie_rejects_chain_without_sql_seg() {
    let chain = make_chain(&["repository", "find"]);
    assert!(detect_scala_doobie_emission(&chain).is_none());
}

#[test]
fn test_scala_quill_query_filter_emits_select_with_type() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let mut chain = make_chain(&["query", "filter"]);
    chain.segments[0].type_args = vec!["User".to_string()];
    match detect_scala_quill_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "scala.User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_scala_zio_sql_select_from_captures_table() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["select", "from"]);
    let args = vec![CallArg::Ident("users".to_string())];
    match detect_scala_zio_sql_emission(&chain, &args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "scala.users");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_scala_zio_sql_rejects_from_without_select() {
    let chain = make_chain(&["someClass", "from"]);
    let args = vec![CallArg::Ident("users".to_string())];
    assert!(detect_scala_zio_sql_emission(&chain, &args).is_none());
}
