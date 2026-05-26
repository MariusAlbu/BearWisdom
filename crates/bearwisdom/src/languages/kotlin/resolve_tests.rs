use super::hooks::{
    detect_kotlin_akka_tell_emission, detect_kotlin_exposed_emission,
    detect_kotlin_grpc_stub_emission, detect_kotlin_ktor_client_emission,
    detect_kotlin_ktor_route_emission,
};
use crate::types::*;

fn make_chain(segments: &[&str]) -> MemberChain {
    MemberChain {
        segments: segments
            .iter()
            .enumerate()
            .map(|(i, name)| ChainSegment {
                name: name.to_string(),
                node_kind: if i == 0 { "identifier".to_string() } else { "navigation_suffix".to_string() },
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
fn test_kotlin_ktor_get_route_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_kotlin_ktor_route_emission("get", &args).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, method, .. } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "/api/users");
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_kotlin_ktor_post_route_emits_post() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/login".to_string())];
    match detect_kotlin_ktor_route_emission("post", &args).unwrap() {
        FlowEmission::NamedChannel { method, .. } => assert_eq!(method, Some(HttpMethod::Post)),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_kotlin_ktor_route_rejects_non_url() {
    let args = vec![CallArg::StringLit("not-a-path".to_string())];
    assert!(detect_kotlin_ktor_route_emission("get", &args).is_none());
}

#[test]
fn test_kotlin_ktor_client_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    let chain = make_chain(&["client", "get"]);
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_kotlin_ktor_client_emission(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, method, .. } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "/api/users");
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_kotlin_exposed_users_select_emits_dbquery() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["Users", "select"]);
    match detect_kotlin_exposed_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "kt.Users");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_kotlin_exposed_insert_emits_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["Posts", "insert"]);
    match detect_kotlin_exposed_emission(&chain).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_kotlin_exposed_rejects_lowercase_root() {
    let chain = make_chain(&["users", "select"]);
    assert!(detect_kotlin_exposed_emission(&chain).is_none());
}

#[test]
fn test_kotlin_grpc_emits_rpc_call() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let chain = make_chain(&["UserServiceGrpc", "newBlockingStub", "getUser"]);
    match detect_kotlin_grpc_stub_emission(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::RpcCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "UserService.getUser");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_kotlin_grpc_rejects_non_grpc_root() {
    let chain = make_chain(&["UserService", "newBlockingStub", "getUser"]);
    assert!(detect_kotlin_grpc_stub_emission(&chain).is_none());
}

#[test]
fn test_kotlin_akka_actor_tell_emits_bgjob() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let chain = make_chain(&["UserActor", "tell"]);
    match detect_kotlin_akka_tell_emission(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::BgJob));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "kt.akka.UserActor");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_kotlin_akka_camelcase_actor_var_recognised() {
    // Local actor variables conventionally end with `Actor` (`myActor`,
    // `userActor`). The root-suffix check accepts both PascalCase types
    // and camelCase var names — both are Akka receivers.
    let chain = make_chain(&["userActor", "tell"]);
    assert!(detect_kotlin_akka_tell_emission(&chain).is_some());
}

#[test]
fn test_kotlin_akka_actor_ask_recognised() {
    let chain = make_chain(&["actor", "ask"]);
    assert!(detect_kotlin_akka_tell_emission(&chain).is_some());
}

#[test]
fn test_kotlin_akka_rejects_non_actor_root() {
    let chain = make_chain(&["repository", "tell"]);
    assert!(detect_kotlin_akka_tell_emission(&chain).is_none());
}
