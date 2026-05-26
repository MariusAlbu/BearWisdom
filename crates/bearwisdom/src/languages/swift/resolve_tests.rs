use super::hooks::{detect_swift_grdb_emission, detect_swift_grpc_emission, detect_swift_http_chain, detect_swift_vapor_route};
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
                is_call: false,
                type_arg_ids: Vec::new(),
})
            .collect(),
    }
}

#[test]
fn test_swift_vapor_get_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    let args = vec![CallArg::StringLit("users".to_string())];
    match detect_swift_vapor_route("get", &args).unwrap() {
        FlowEmission::NamedChannel { kind, role, method, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
            assert_eq!(name, "/users");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_swift_vapor_post_works() {
    let args = vec![CallArg::StringLit("/api/items".to_string())];
    assert!(detect_swift_vapor_route("post", &args).is_some());
}

#[test]
fn test_swift_vapor_rejects_unknown_verb() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_swift_vapor_route("middleware", &args).is_none());
}

#[test]
fn test_swift_alamofire_request_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let chain = make_chain(&["AF", "request"]);
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_swift_http_chain(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { kind, role, .. } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Producer);
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_swift_http_rejects_other_root() {
    let chain = make_chain(&["logger", "request"]);
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_swift_http_chain(&chain, &args).is_none());
}

#[test]
fn test_swift_grdb_user_fetch_all_emits_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["User", "fetchAll"]);
    match detect_swift_grdb_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "swift.User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_swift_grdb_rejects_lowercase_root() {
    let chain = make_chain(&["user", "fetchAll"]);
    assert!(detect_swift_grdb_emission(&chain).is_none());
}

#[test]
fn test_swift_grpc_emits_rpc_call() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let chain = make_chain(&["UserServiceClient", "getUser"]);
    match detect_swift_grpc_emission(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::RpcCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "UserService.getUser");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_swift_grpc_rejects_non_client() {
    let chain = make_chain(&["UserService", "getUser"]);
    assert!(detect_swift_grpc_emission(&chain).is_none());
}
