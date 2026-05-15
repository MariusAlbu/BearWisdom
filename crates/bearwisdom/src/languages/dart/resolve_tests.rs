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
            })
            .collect(),
    }
}

#[test]
fn test_dart_shelf_route_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_dart_shelf_route("get", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_dart_shelf_post_works() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_dart_shelf_route("post", &args).is_some());
}

#[test]
fn test_dart_shelf_rejects_unknown() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_dart_shelf_route("middleware", &args).is_none());
}

#[test]
fn test_dart_dio_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    let chain = make_chain(&["dio", "get"]);
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_dart_http_chain(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { role, .. } => assert_eq!(role, ChannelRole::Producer),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_dart_http_rejects_other_root() {
    let chain = make_chain(&["myObj", "get"]);
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_dart_http_chain(&chain, &args).is_none());
}

#[test]
fn test_dart_drift_select_emits_db_query() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["select", "get"]);
    match detect_dart_drift_emission(&chain).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_dart_drift_update_emits_update() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["update", "write"]);
    match detect_dart_drift_emission(&chain).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Update),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_dart_grpc_emits_rpc_call() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let chain = make_chain(&["UserServiceClient", "getUser"]);
    match detect_dart_grpc_emission(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::RpcCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "UserService.getUser");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_dart_grpc_rejects_non_client() {
    let chain = make_chain(&["UserService", "getUser"]);
    assert!(detect_dart_grpc_emission(&chain).is_none());
}
