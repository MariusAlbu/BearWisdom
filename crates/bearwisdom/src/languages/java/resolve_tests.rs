use crate::indexer::resolve::engine::SymbolIndex;
use crate::types::*;
use std::collections::HashMap;

fn make_file(path: &str, lang: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: lang.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    }
}

fn build_test_env(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in files {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }
    let owned: Vec<ParsedFile> = files
        .iter()
        .map(|f| ParsedFile {
            path: f.path.clone(),
            language: f.language.clone(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            content: None,
            has_errors: false,
            symbols: f.symbols.clone(),
            refs: f.refs.clone(),
            routes: vec![],
            db_sets: vec![],
            symbol_origin_languages: vec![],
            ref_origin_languages: vec![],
            symbol_from_snippet: vec![],
            flow: crate::types::FlowMeta::default(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
            component_selectors: Vec::new(),

            plugin_flow_emissions: Vec::new(),
        })
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

// ---------------------------------------------------------------------------
// Lombok synthesis → index type-info tests
// ---------------------------------------------------------------------------

/// Extract + Lombok-synthesize + splice (rebasing synthesized refs) the way
/// `parse_file` does, into a single ParsedFile.
fn synth_splice(path: &str, source: &str) -> ParsedFile {
    let r = super::extract::extract(source);
    let s = super::lombok::synthesize_lombok_accessors(source, &r.symbols, &r.refs);
    let base = r.symbols.len();
    let mut symbols = r.symbols.clone();
    symbols.extend(s.symbols);
    let mut refs = r.refs.clone();
    for mut sref in s.refs {
        sref.source_symbol_index += base;
        refs.push(sref);
    }
    make_file(path, "java", symbols, refs)
}

#[test]
fn lombok_synthesized_methods_carry_return_types_in_index() {
    use crate::indexer::resolve::engine::SymbolLookup;

    // The synthesized return-type refs flow through the index builder into the
    // return-type map, so a chain types through builder()/getName()/build() the
    // same way it does through a hand-written method.
    let user = synth_splice(
        "src/User.java",
        "@Data\n@Builder\npublic class User { private String name; }",
    );
    let (index, _id_map) = build_test_env(&[&user]);

    assert_eq!(index.return_type_name("User.getName"), Some("String"));
    assert_eq!(index.return_type_name("User.builder"), Some("User.UserBuilder"));
    assert_eq!(index.return_type_name("User.UserBuilder.name"), Some("User.UserBuilder"));
    assert_eq!(index.return_type_name("User.UserBuilder.build"), Some("User"));
    // Void setter: no return type.
    assert_eq!(index.return_type_name("User.setName"), None);
}

#[test]
fn lombok_generic_getter_binds_element_via_signature() {
    use crate::indexer::resolve::engine::SymbolLookup;

    // A generic getter's synthesized leading-form signature (`List<User>
    // getItems()`) lets the index recover the element args, so the getter
    // carries return_type_args=["User"] and a chain types through to the element.
    let order = synth_splice(
        "src/Order.java",
        "@Data\npublic class Order { private List<User> items; }",
    );
    let (index, _id_map) = build_test_env(&[&order]);

    assert_eq!(index.return_type_name("Order.getItems"), Some("List"));
    assert_eq!(
        index.return_type_args("Order.getItems").map(|a| a.to_vec()),
        Some(vec!["User".to_string()])
    );
}

// ---------------------------------------------------------------------------
// HTTP Producer + DbQuery flow detection (Goal 13)
// ---------------------------------------------------------------------------

fn make_chain(segments: &[&str]) -> MemberChain {
    MemberChain {
        segments: segments
            .iter()
            .enumerate()
            .map(|(i, name)| ChainSegment {
                name: name.to_string(),
                node_kind: if i == 0 {
                    "identifier".to_string()
                } else {
                    "property_identifier".to_string()
                },
                kind: if i == 0 {
                    SegmentKind::Identifier
                } else {
                    SegmentKind::Property
                },
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
fn test_java_resttemplate_get_for_object_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    use super::hooks::detect_java_http_chain_emission;

    let chain = make_chain(&["restTemplate", "getForObject"]);
    let call_args = vec![
        CallArg::StringLit("/api/users/{id}".to_string()),
        CallArg::Ident("User".to_string()),
    ];
    match detect_java_http_chain_emission(&chain, &call_args).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, method, .. } => {
            assert_eq!(kind, NamedChannelKind::HttpCall);
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "/api/users/{}");
            assert_eq!(method, Some(HttpMethod::Get));
        }
        other => panic!("expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn test_java_resttemplate_post_for_entity_emits_post() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_java_http_chain_emission;

    let chain = make_chain(&["restTemplate", "postForEntity"]);
    let call_args = vec![
        CallArg::StringLit("/api/login".to_string()),
        CallArg::Ident("payload".to_string()),
        CallArg::Ident("LoginResponse".to_string()),
    ];
    match detect_java_http_chain_emission(&chain, &call_args).unwrap() {
        FlowEmission::NamedChannel { method, name, .. } => {
            assert_eq!(method, Some(HttpMethod::Post));
            assert_eq!(name, "/api/login");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_java_webclient_get_uri_emits_producer() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_java_http_chain_emission;

    let chain = make_chain(&["webClient", "get", "uri"]);
    let call_args = vec![CallArg::StringLit("/api/things".to_string())];
    match detect_java_http_chain_emission(&chain, &call_args).unwrap() {
        FlowEmission::NamedChannel { method, name, .. } => {
            assert_eq!(method, Some(HttpMethod::Get));
            assert_eq!(name, "/api/things");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_java_okhttp_url_emits_any_method() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_java_http_chain_emission;

    let chain = make_chain(&["builder", "url"]);
    let call_args = vec![CallArg::StringLit("https://api.example.com/x".to_string())];
    match detect_java_http_chain_emission(&chain, &call_args).unwrap() {
        FlowEmission::NamedChannel { method, name, .. } => {
            assert_eq!(method, Some(HttpMethod::Any));
            assert_eq!(name, "/x");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_java_http_no_emit_for_unknown_verb() {
    use super::hooks::detect_java_http_chain_emission;

    let chain = make_chain(&["restTemplate", "doSomething"]);
    let call_args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_java_http_chain_emission(&chain, &call_args).is_none());
}

#[test]
fn test_java_http_no_emit_when_url_is_variable() {
    use super::hooks::detect_java_http_chain_emission;

    let chain = make_chain(&["restTemplate", "getForObject"]);
    let call_args = vec![CallArg::Ident("url".to_string()), CallArg::Ident("Object".to_string())];
    assert!(detect_java_http_chain_emission(&chain, &call_args).is_none());
}

#[test]
fn test_java_jpa_find_emits_dbquery() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_java_db_query_emission;

    let chain = make_chain(&["entityManager", "find"]);
    let call_args = vec![CallArg::Ident("User".to_string()), CallArg::Literal("1".to_string())];
    match detect_java_db_query_emission(&chain, &call_args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "java.User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_java_jpa_create_query_emits_dbquery() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_java_db_query_emission;

    let chain = make_chain(&["entityManager", "createQuery"]);
    let call_args = vec![
        CallArg::StringLit("FROM Poll p".to_string()),
        CallArg::Ident("Poll".to_string()),
    ];
    match detect_java_db_query_emission(&chain, &call_args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "java.Poll");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_java_jpa_no_emit_without_entity_class() {
    use super::hooks::detect_java_db_query_emission;

    let chain = make_chain(&["entityManager", "find"]);
    let call_args = vec![CallArg::Ident("entity".to_string())];
    assert!(detect_java_db_query_emission(&chain, &call_args).is_none());
}

#[test]
fn test_java_jpa_no_emit_for_unknown_method() {
    use super::hooks::detect_java_db_query_emission;

    let chain = make_chain(&["entityManager", "flush"]);
    let call_args: Vec<CallArg> = vec![];
    assert!(detect_java_db_query_emission(&chain, &call_args).is_none());
}

#[test]
fn test_java_spring_data_query_annotation_emits_dbquery() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_jpa_query_annotation_emission;

    let sql = "SELECT u FROM User u WHERE u.email = :email";
    match detect_jpa_query_annotation_emission("Query", Some(sql)).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "java.User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_java_query_annotation_no_emit_for_other_annotations() {
    use super::hooks::detect_jpa_query_annotation_emission;

    assert!(detect_jpa_query_annotation_emission("GetMapping", Some("/x")).is_none());
    assert!(detect_jpa_query_annotation_emission("Service", None).is_none());
    assert!(detect_jpa_query_annotation_emission("Query", None).is_none());
    assert!(detect_jpa_query_annotation_emission("Query", Some("DROP TABLE x")).is_none());
}

// ---------------------------------------------------------------------------
// Goal 20 — JdbcTemplate + Retrofit + grpc-java
// ---------------------------------------------------------------------------

#[test]
fn test_java_jdbc_template_query_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_java_jdbc_template_emission;

    let chain = make_chain(&["jdbcTemplate", "query"]);
    let args = vec![CallArg::StringLit("SELECT id FROM users WHERE x = ?".to_string()), CallArg::Other];
    match detect_java_jdbc_template_emission(&chain, &args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "java.users");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_java_jdbc_template_update() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_java_jdbc_template_emission;

    let chain = make_chain(&["jdbcTemplate", "update"]);
    let args = vec![CallArg::StringLit("UPDATE accounts SET balance = ? WHERE id = ?".to_string())];
    match detect_java_jdbc_template_emission(&chain, &args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "java.accounts");
            assert_eq!(operation, DbQueryOp::Update);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_java_jdbc_template_rejects_non_template_root() {
    use super::hooks::detect_java_jdbc_template_emission;

    let chain = make_chain(&["someService", "query"]);
    let args = vec![CallArg::StringLit("SELECT * FROM x".to_string())];
    assert!(detect_java_jdbc_template_emission(&chain, &args).is_none());
}

#[test]
fn test_java_retrofit_get_attribute_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    use super::hooks::detect_retrofit_attribute_emission;

    match detect_retrofit_attribute_emission("GET", Some("/api/users")).unwrap() {
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
fn test_java_retrofit_post_emits_post_method() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_retrofit_attribute_emission;

    match detect_retrofit_attribute_emission("POST", Some("/login")).unwrap() {
        FlowEmission::NamedChannel { method, .. } => assert_eq!(method, Some(HttpMethod::Post)),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_java_retrofit_rejects_non_verb_attribute() {
    use super::hooks::detect_retrofit_attribute_emission;

    assert!(detect_retrofit_attribute_emission("Service", Some("/x")).is_none());
    assert!(detect_retrofit_attribute_emission("GET", None).is_none());
    assert!(detect_retrofit_attribute_emission("GET", Some("")).is_none());
}

#[test]
fn test_java_grpc_stub_emits_rpc_call() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_java_grpc_stub_emission;

    let chain = make_chain(&["UserServiceGrpc", "newBlockingStub", "getUser"]);
    match detect_java_grpc_stub_emission(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::RpcCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "UserService.getUser");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_java_grpc_future_stub_works() {
    use super::hooks::detect_java_grpc_stub_emission;

    let chain = make_chain(&["HelloServiceGrpc", "newFutureStub", "sayHello"]);
    assert!(detect_java_grpc_stub_emission(&chain).is_some());
}

#[test]
fn test_java_grpc_rejects_non_grpc_root() {
    use super::hooks::detect_java_grpc_stub_emission;

    let chain = make_chain(&["UserService", "newBlockingStub", "getUser"]);
    assert!(detect_java_grpc_stub_emission(&chain).is_none());
}

#[test]
fn test_java_quartz_schedule_emits_bgjob() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_java_quartz_emission;
    let chain = make_chain(&["scheduler", "scheduleJob"]);
    match detect_java_quartz_emission(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::BgJob));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "java.quartz");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_java_kafka_template_send_emits_mq_with_topic() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_java_jms_kafka_emission;
    let chain = make_chain(&["kafkaTemplate", "send"]);
    let args = vec![
        CallArg::StringLit("user.created".to_string()),
        CallArg::Ident("payload".to_string()),
    ];
    match detect_java_jms_kafka_emission(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { kind, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::MessageQueue));
            assert_eq!(name, "user.created");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_java_jms_template_send_emits_mq() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_java_jms_kafka_emission;
    let chain = make_chain(&["jmsTemplate", "send"]);
    match detect_java_jms_kafka_emission(&chain, &[]).unwrap() {
        FlowEmission::NamedChannel { kind, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::MessageQueue));
            assert_eq!(name, "java.jms");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_java_redis_template_get_emits_config_lookup() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_java_redis_template_emission;
    let chain = make_chain(&["redisTemplate", "opsForValue", "get"]);
    let args = vec![CallArg::StringLit("feature:flag".to_string())];
    match detect_java_redis_template_emission(&chain, &args).unwrap() {
        FlowEmission::ConfigLookup { key } => assert_eq!(key, "redis:feature:flag"),
        _ => panic!("expected ConfigLookup"),
    }
}

#[test]
fn test_java_message_mapping_emits_ws_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_java_message_mapping_emission;
    match detect_java_message_mapping_emission("MessageMapping", Some("/chat/{room}")).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::WebSocket));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "/chat/{room}");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_java_message_mapping_rejects_other_annotations() {
    use super::hooks::detect_java_message_mapping_emission;
    assert!(detect_java_message_mapping_emission("Component", None).is_none());
}

#[test]
fn test_java_jakarta_server_endpoint_with_path() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_java_message_mapping_emission;
    match detect_java_message_mapping_emission("ServerEndpoint", Some("/ws/chat")).unwrap() {
        FlowEmission::NamedChannel { kind, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::WebSocket));
            assert_eq!(name, "/ws/chat");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_java_jakarta_on_open_recognised() {
    use super::hooks::detect_java_message_mapping_emission;
    assert!(detect_java_message_mapping_emission("OnOpen", None).is_some());
    assert!(detect_java_message_mapping_emission("OnClose", None).is_some());
    assert!(detect_java_message_mapping_emission("OnError", None).is_some());
}
