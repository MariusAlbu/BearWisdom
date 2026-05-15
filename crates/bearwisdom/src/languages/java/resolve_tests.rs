use super::resolve::JavaResolver;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{build_scope_chain, FileContext, LanguageResolver, RefContext, SymbolIndex, SymbolInfo};
use crate::types::*;
use std::collections::HashMap;

fn make_symbol(
    name: &str,
    qname: &str,
    kind: SymbolKind,
    vis: Visibility,
    scope: Option<&str>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(vis),
        start_line: 1,
        end_line: 10,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: scope.map(|s| s.to_string()),
        parent_index: None,
    }
}

fn make_ref(source_idx: usize, target: &str, kind: EdgeKind, line: u32) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind,
        line,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}
fn make_import_ref(source_idx: usize, name: &str, module: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: source_idx,
        target_name: name.to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        module: Some(module.to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}
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
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_scope_chain_resolution() {
    let file = make_file(
        "src/OrderService.java",
        "java",
        vec![
            make_symbol("com.example", "com.example", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("OrderService", "com.example.OrderService", SymbolKind::Class, Visibility::Public, Some("com.example")),
            make_symbol("create", "com.example.OrderService.create", SymbolKind::Method, Visibility::Public, Some("com.example.OrderService")),
            make_symbol("validate", "com.example.OrderService.validate", SymbolKind::Method, Visibility::Private, Some("com.example.OrderService")),
        ],
        vec![make_ref(2, "validate", EdgeKind::Calls, 10)],
    );

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = JavaResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[2],
        scope_chain: build_scope_chain(file.symbols[2].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve validate via scope chain");
    let res = result.unwrap();
    assert_eq!(res.strategy, "java_scope_chain");
    assert_eq!(res.confidence, 1.0);
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("src/OrderService.java".to_string(), "com.example.OrderService.validate".to_string())).unwrap()
    );
}

#[test]
fn test_same_package_resolution() {
    let file1 = make_file(
        "src/Order.java",
        "java",
        vec![
            make_symbol("com.example", "com.example", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("Order", "com.example.Order", SymbolKind::Class, Visibility::Public, Some("com.example")),
        ],
        vec![],
    );

    let file2 = make_file(
        "src/OrderService.java",
        "java",
        vec![
            make_symbol("com.example", "com.example", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("OrderService", "com.example.OrderService", SymbolKind::Class, Visibility::Public, Some("com.example")),
        ],
        // No import — same package visibility
        vec![make_ref(1, "Order", EdgeKind::TypeRef, 5)],
    );

    let (index, id_map) = build_test_env(&[&file1, &file2]);
    let resolver = JavaResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[1],
        scope_chain: build_scope_chain(file2.symbols[1].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve Order via same-package");
    let res = result.unwrap();
    assert!(
        res.strategy == "java_scope_chain" || res.strategy == "java_same_package",
        "Unexpected strategy: {}",
        res.strategy
    );
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("src/Order.java".to_string(), "com.example.Order".to_string())).unwrap()
    );
}

#[test]
fn test_exact_import_resolution() {
    let file1 = make_file(
        "src/Product.java",
        "java",
        vec![make_symbol("Product", "com.store.model.Product", SymbolKind::Class, Visibility::Public, Some("com.store.model"))],
        vec![],
    );

    let file2 = make_file(
        "src/ProductController.java",
        "java",
        vec![make_symbol("ProductController", "com.store.web.ProductController", SymbolKind::Class, Visibility::Public, Some("com.store.web"))],
        vec![
            make_ref(0, "Product", EdgeKind::TypeRef, 10),
            make_import_ref(0, "Product", "com.store.model.Product"),
        ],
    );

    let (index, id_map) = build_test_env(&[&file1, &file2]);
    let resolver = JavaResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[0],
        scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve Product via import");
    let res = result.unwrap();
    assert_eq!(res.strategy, "java_import");
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("src/Product.java".to_string(), "com.store.model.Product".to_string())).unwrap()
    );
}

#[test]
fn test_wildcard_import_resolution() {
    let file1 = make_file(
        "src/User.java",
        "java",
        vec![make_symbol("User", "com.app.model.User", SymbolKind::Class, Visibility::Public, Some("com.app.model"))],
        vec![],
    );

    let file2 = make_file(
        "src/UserService.java",
        "java",
        vec![make_symbol("UserService", "com.app.service.UserService", SymbolKind::Class, Visibility::Public, Some("com.app.service"))],
        vec![
            make_ref(0, "User", EdgeKind::TypeRef, 5),
            // Wildcard import: import com.app.model.*;
            make_import_ref(0, "*", "com.app.model"),
        ],
    );

    let (index, id_map) = build_test_env(&[&file1, &file2]);
    let resolver = JavaResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[0],
        scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "Should resolve User via wildcard import");
    let res = result.unwrap();
    assert_eq!(res.strategy, "java_wildcard_import");
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("src/User.java".to_string(), "com.app.model.User".to_string())).unwrap()
    );
}

#[test]
fn test_private_cross_file_not_resolved() {
    let file1 = make_file(
        "src/Internal.java",
        "java",
        vec![make_symbol("helper", "com.app.Internal.helper", SymbolKind::Method, Visibility::Private, Some("com.app.Internal"))],
        vec![],
    );

    let file2 = make_file(
        "src/Client.java",
        "java",
        vec![make_symbol("Client", "com.app.Client", SymbolKind::Class, Visibility::Public, Some("com.app"))],
        vec![
            make_ref(0, "helper", EdgeKind::Calls, 5),
            make_import_ref(0, "*", "com.app"),
        ],
    );

    let (index, _) = build_test_env(&[&file1, &file2]);
    let resolver = JavaResolver;
    let file_ctx = resolver.build_file_context(&file2, None);

    let ref_ctx = RefContext {
        extracted_ref: &file2.refs[0],
        source_symbol: &file2.symbols[0],
        scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    // Private cross-file should not resolve.
    assert!(
        resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "Private cross-file should not resolve"
    );
}

#[test]
fn test_falls_back_for_unknown() {
    let file = make_file(
        "src/Test.java",
        "java",
        vec![make_symbol("Test", "com.app.Test", SymbolKind::Class, Visibility::Public, Some("com.app"))],
        vec![make_ref(0, "Nonexistent", EdgeKind::TypeRef, 5)],
    );

    let (index, _) = build_test_env(&[&file]);
    let resolver = JavaResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
    file_package_id: None,
    };

    assert!(
        resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(),
        "Unknown symbol should fall back"
    );
}

#[test]
fn test_build_file_context_extracts_package() {
    let file = make_file(
        "src/Foo.java",
        "java",
        vec![
            make_symbol("com.example", "com.example", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("Foo", "com.example.Foo", SymbolKind::Class, Visibility::Public, Some("com.example")),
        ],
        vec![],
    );

    let resolver = JavaResolver;
    let ctx = resolver.build_file_context(&file, None);
    assert_eq!(ctx.file_namespace, Some("com.example".to_string()));
}

#[test]
fn test_build_file_context_wildcard_import() {
    let file = make_file(
        "src/Foo.java",
        "java",
        vec![make_symbol("Foo", "com.example.Foo", SymbolKind::Class, Visibility::Public, Some("com.example"))],
        vec![make_import_ref(0, "*", "org.springframework.web.bind.annotation")],
    );

    let resolver = JavaResolver;
    let ctx = resolver.build_file_context(&file, None);
    assert_eq!(ctx.imports.len(), 1);
    assert!(ctx.imports[0].is_wildcard);
    assert_eq!(
        ctx.imports[0].module_path.as_deref(),
        Some("org.springframework.web.bind.annotation")
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
            })
            .collect(),
    }
}

#[test]
fn test_java_resttemplate_get_for_object_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    use super::resolve::detect_java_http_chain_emission;

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
    use super::resolve::detect_java_http_chain_emission;

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
    use super::resolve::detect_java_http_chain_emission;

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
    use super::resolve::detect_java_http_chain_emission;

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
    use super::resolve::detect_java_http_chain_emission;

    let chain = make_chain(&["restTemplate", "doSomething"]);
    let call_args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_java_http_chain_emission(&chain, &call_args).is_none());
}

#[test]
fn test_java_http_no_emit_when_url_is_variable() {
    use super::resolve::detect_java_http_chain_emission;

    let chain = make_chain(&["restTemplate", "getForObject"]);
    let call_args = vec![CallArg::Ident("url".to_string()), CallArg::Ident("Object".to_string())];
    assert!(detect_java_http_chain_emission(&chain, &call_args).is_none());
}

#[test]
fn test_java_jpa_find_emits_dbquery() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::resolve::detect_java_db_query_emission;

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
    use super::resolve::detect_java_db_query_emission;

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
    use super::resolve::detect_java_db_query_emission;

    let chain = make_chain(&["entityManager", "find"]);
    let call_args = vec![CallArg::Ident("entity".to_string())];
    assert!(detect_java_db_query_emission(&chain, &call_args).is_none());
}

#[test]
fn test_java_jpa_no_emit_for_unknown_method() {
    use super::resolve::detect_java_db_query_emission;

    let chain = make_chain(&["entityManager", "flush"]);
    let call_args: Vec<CallArg> = vec![];
    assert!(detect_java_db_query_emission(&chain, &call_args).is_none());
}

#[test]
fn test_java_spring_data_query_annotation_emits_dbquery() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::resolve::detect_jpa_query_annotation_emission;

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
    use super::resolve::detect_jpa_query_annotation_emission;

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
    use super::resolve::detect_java_jdbc_template_emission;

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
    use super::resolve::detect_java_jdbc_template_emission;

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
    use super::resolve::detect_java_jdbc_template_emission;

    let chain = make_chain(&["someService", "query"]);
    let args = vec![CallArg::StringLit("SELECT * FROM x".to_string())];
    assert!(detect_java_jdbc_template_emission(&chain, &args).is_none());
}

#[test]
fn test_java_retrofit_get_attribute_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    use super::resolve::detect_retrofit_attribute_emission;

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
    use super::resolve::detect_retrofit_attribute_emission;

    match detect_retrofit_attribute_emission("POST", Some("/login")).unwrap() {
        FlowEmission::NamedChannel { method, .. } => assert_eq!(method, Some(HttpMethod::Post)),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_java_retrofit_rejects_non_verb_attribute() {
    use super::resolve::detect_retrofit_attribute_emission;

    assert!(detect_retrofit_attribute_emission("Service", Some("/x")).is_none());
    assert!(detect_retrofit_attribute_emission("GET", None).is_none());
    assert!(detect_retrofit_attribute_emission("GET", Some("")).is_none());
}

#[test]
fn test_java_grpc_stub_emits_rpc_call() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::resolve::detect_java_grpc_stub_emission;

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
    use super::resolve::detect_java_grpc_stub_emission;

    let chain = make_chain(&["HelloServiceGrpc", "newFutureStub", "sayHello"]);
    assert!(detect_java_grpc_stub_emission(&chain).is_some());
}

#[test]
fn test_java_grpc_rejects_non_grpc_root() {
    use super::resolve::detect_java_grpc_stub_emission;

    let chain = make_chain(&["UserService", "newBlockingStub", "getUser"]);
    assert!(detect_java_grpc_stub_emission(&chain).is_none());
}

#[test]
fn test_java_quartz_schedule_emits_bgjob() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::resolve::detect_java_quartz_emission;
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
    use super::resolve::detect_java_jms_kafka_emission;
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
    use super::resolve::detect_java_jms_kafka_emission;
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
    use super::resolve::detect_java_redis_template_emission;
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
    use super::resolve::detect_java_message_mapping_emission;
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
    use super::resolve::detect_java_message_mapping_emission;
    assert!(detect_java_message_mapping_emission("Component", None).is_none());
}

#[test]
fn test_java_jakarta_server_endpoint_with_path() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::resolve::detect_java_message_mapping_emission;
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
    use super::resolve::detect_java_message_mapping_emission;
    assert!(detect_java_message_mapping_emission("OnOpen", None).is_some());
    assert!(detect_java_message_mapping_emission("OnClose", None).is_some());
    assert!(detect_java_message_mapping_emission("OnError", None).is_some());
}
