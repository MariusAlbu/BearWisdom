// =============================================================================
// languages/python/flow_tests.rs — unit tests for flow_detectors.rs
//
// Kept in a sibling file so the production module stays free of synthetic
// fixture literals (dep names, file paths) that look like hardcoded
// production values to a casual reader.
// =============================================================================

use super::flow_detectors::{
    detect_python_channels_consumer_inheritance, detect_python_channels_path_emission,
    detect_python_cursor_execute_emission, detect_python_db_query_emission,
    detect_python_django_path_emission, detect_python_graphql_decorator_emission,
    detect_python_grpc_stub_emission, detect_python_http_chain_emission,
    detect_python_route_decorator_emission, detect_python_sqlalchemy_select_call,
};
use crate::indexer::resolve::engine::{FileContext, ImportEntry};
use crate::types::{CallArg, ChainSegment, MemberChain, SegmentKind};

fn make_chain(segments: &[&str]) -> MemberChain {
    MemberChain {
        segments: segments
            .iter()
            .enumerate()
            .map(|(i, name)| ChainSegment {
                name: name.to_string(),
                node_kind: if i == 0 { "identifier".to_string() } else { "property_identifier".to_string() },
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

fn make_file_ctx_with_imports(libs: &[&str]) -> FileContext {
    FileContext {
        file_path: "src/api.py".to_string(),
        language: "python".to_string(),
        imports: libs
            .iter()
            .map(|lib| ImportEntry {
                imported_name: lib.to_string(),
                module_path: Some(lib.to_string()),
                alias: None,
                is_wildcard: false,
            })
            .collect(),
        file_namespace: None,
    }
}

#[test]
fn fastapi_get_decorator_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    match detect_python_route_decorator_emission("app.get", Some("/users/{id}")).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, method, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "/users/{}");
            assert_eq!(method, Some(HttpMethod::Get));
        }
        other => panic!("expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn fastapi_router_post_emits_post() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    match detect_python_route_decorator_emission("router.post", Some("/login")).unwrap() {
        FlowEmission::NamedChannel { method, name, .. } => {
            assert_eq!(method, Some(HttpMethod::Post));
            assert_eq!(name, "/login");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn flask_route_decorator_emits_any() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    match detect_python_route_decorator_emission("app.route", Some("/x")).unwrap() {
        FlowEmission::NamedChannel { method, .. } => {
            assert_eq!(method, Some(HttpMethod::Any));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn route_decorator_no_emit_for_unrelated_decorator() {
    assert!(detect_python_route_decorator_emission("dataclass", None).is_none());
    assert!(detect_python_route_decorator_emission("classmethod", None).is_none());
    assert!(detect_python_route_decorator_emission("app.get", None).is_none());
}

#[test]
fn requests_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    let chain = make_chain(&["requests", "get"]);
    let call_args = vec![CallArg::StringLit("/api/users".to_string())];
    let ctx = make_file_ctx_with_imports(&["requests"]);
    match detect_python_http_chain_emission(&chain, &call_args, &ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, method, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "/api/users");
            assert_eq!(method, Some(HttpMethod::Get));
        }
        other => panic!("expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn httpx_async_client_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    let chain = make_chain(&["client", "get"]);
    let call_args = vec![CallArg::StringLit("/api/me".to_string())];
    let ctx = make_file_ctx_with_imports(&["httpx"]);
    match detect_python_http_chain_emission(&chain, &call_args, &ctx).unwrap() {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/api/me"),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn aiohttp_session_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    let chain = make_chain(&["session", "get"]);
    let call_args = vec![CallArg::StringLit("/api/data".to_string())];
    let ctx = make_file_ctx_with_imports(&["aiohttp"]);
    match detect_python_http_chain_emission(&chain, &call_args, &ctx).unwrap() {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/api/data"),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn http_producer_no_emit_without_library_import() {
    let chain = make_chain(&["obj", "get"]);
    let call_args = vec![CallArg::StringLit("key".to_string())];
    let ctx = make_file_ctx_with_imports(&["typing"]);
    assert!(detect_python_http_chain_emission(&chain, &call_args, &ctx).is_none());
}

#[test]
fn sqlalchemy_entity_query_filter_emits_dbquery() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["User", "query", "filter"]);
    match detect_python_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "py.User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        other => panic!("expected DbQuery, got {other:?}"),
    }
}

#[test]
fn django_entity_objects_create_emits_dbquery_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["User", "objects", "create"]);
    match detect_python_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "py.User");
            assert_eq!(operation, DbQueryOp::Insert);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn django_entity_objects_filter_emits_dbquery_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["Poll", "objects", "filter"]);
    match detect_python_db_query_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "py.Poll");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn db_query_no_emit_for_unrelated_chain() {
    let chain = make_chain(&["some", "thing", "method"]);
    assert!(detect_python_db_query_emission(&chain).is_none());
    let chain2 = make_chain(&["user", "objects", "all"]);
    assert!(detect_python_db_query_emission(&chain2).is_none());
}

// -----------------------------------------------------------------------
// Goal 19 — extended detectors
// -----------------------------------------------------------------------

#[test]
fn django_path_emits_consumer_http() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    let args = vec![CallArg::StringLit("users/".to_string()), CallArg::Other];
    let ctx = make_file_ctx_with_imports(&["django"]);
    match detect_python_django_path_emission("path", &args, &ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, method, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "/users");
            assert_eq!(method, Some(HttpMethod::Any));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn django_re_path_strips_anchors() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    let args = vec![CallArg::StringLit("^users/$".to_string())];
    let ctx = make_file_ctx_with_imports(&["django"]);
    match detect_python_django_path_emission("re_path", &args, &ctx).unwrap() {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/users"),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn django_path_requires_django_import() {
    let args = vec![CallArg::StringLit("users/".to_string())];
    let ctx = make_file_ctx_with_imports(&["typing"]);
    assert!(detect_python_django_path_emission("path", &args, &ctx).is_none());
}

#[test]
fn channels_consumer_inheritance_emits_ws() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    match detect_python_channels_consumer_inheritance("AsyncWebsocketConsumer").unwrap() {
        FlowEmission::NamedChannel { kind, role, .. } => {
            assert!(matches!(kind, NamedChannelKind::WebSocket));
            assert_eq!(role, ChannelRole::Consumer);
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn channels_json_consumer_also_recognised() {
    assert!(detect_python_channels_consumer_inheritance("JsonWebsocketConsumer").is_some());
    assert!(detect_python_channels_consumer_inheritance("AsyncJsonWebsocketConsumer").is_some());
}

#[test]
fn channels_rejects_non_consumer_base() {
    assert!(detect_python_channels_consumer_inheritance("View").is_none());
    assert!(detect_python_channels_consumer_inheritance("models.Model").is_none());
}

#[test]
fn channels_path_emits_ws_when_channels_imported() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let args = vec![CallArg::StringLit("ws/chat/".to_string())];
    let ctx = make_file_ctx_with_imports(&["channels"]);
    match detect_python_channels_path_emission("path", &args, &ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::WebSocket));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "/ws/chat");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn channels_path_requires_channels_import() {
    let args = vec![CallArg::StringLit("ws/chat/".to_string())];
    let ctx = make_file_ctx_with_imports(&["django"]);
    assert!(detect_python_channels_path_emission("path", &args, &ctx).is_none());
}

#[test]
fn strawberry_field_emits_graphql_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    match detect_python_graphql_decorator_emission("strawberry.field", "users").unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::GraphQLOp));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "field:users");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn graphene_mutation_recognised() {
    assert!(
        detect_python_graphql_decorator_emission("graphene.mutation", "createUser").is_some()
    );
}

#[test]
fn graphql_rejects_non_op_decorator() {
    // `@strawberry.type` marks a schema type, not a callable op — skip
    // until DbEntity-style pairing is wired for GraphQL types.
    assert!(detect_python_graphql_decorator_emission("strawberry.type", "User").is_none());
}

#[test]
fn graphql_rejects_unrelated_decorator() {
    assert!(detect_python_graphql_decorator_emission("functools.cache", "f").is_none());
}

#[test]
fn urllib_urlopen_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    let chain = make_chain(&["urllib", "request", "urlopen"]);
    let args = vec![CallArg::StringLit("https://api.example.com/x".to_string())];
    let ctx = make_file_ctx_with_imports(&["urllib"]);
    match detect_python_http_chain_emission(&chain, &args, &ctx).unwrap() {
        FlowEmission::NamedChannel { kind, role, method, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(method, Some(HttpMethod::Any));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn sqlalchemy_select_call_emits_db_query() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let args = vec![CallArg::Ident("User".to_string())];
    let ctx = make_file_ctx_with_imports(&["sqlalchemy"]);
    match detect_python_sqlalchemy_select_call("select", &args, &ctx).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "py.User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn sqlalchemy_select_requires_sqlalchemy_import() {
    let args = vec![CallArg::Ident("User".to_string())];
    let ctx = make_file_ctx_with_imports(&["typing"]);
    assert!(detect_python_sqlalchemy_select_call("select", &args, &ctx).is_none());
}

#[test]
fn cursor_execute_select_emits_db_query() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["cursor", "execute"]);
    let args = vec![CallArg::StringLit("SELECT id FROM users WHERE active = 1".to_string())];
    match detect_python_cursor_execute_emission(&chain, &args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "py.users");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn cursor_execute_insert_emits_op() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["cursor", "execute"]);
    let args = vec![CallArg::StringLit("INSERT INTO items (a) VALUES (1)".to_string())];
    match detect_python_cursor_execute_emission(&chain, &args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "py.items");
            assert_eq!(operation, DbQueryOp::Insert);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn grpc_stub_method_emits_rpc_call() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let chain = make_chain(&["UserServiceStub", "GetUser"]);
    match detect_python_grpc_stub_emission(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::RpcCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "UserService.GetUser");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn grpc_stub_rejects_non_stub_root() {
    let chain = make_chain(&["UserService", "GetUser"]);
    assert!(detect_python_grpc_stub_emission(&chain).is_none());
}
