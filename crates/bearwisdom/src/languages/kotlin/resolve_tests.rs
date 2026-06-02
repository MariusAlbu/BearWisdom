use super::hooks::{
    detect_kotlin_akka_tell_emission, detect_kotlin_exposed_emission,
    detect_kotlin_grpc_stub_emission, detect_kotlin_ktor_client_emission,
    detect_kotlin_ktor_route_emission, KotlinResolver,
};
use crate::indexer::resolve::engine::{RefContext, SymbolIndex};
use crate::types::*;
use std::collections::HashMap;

fn make_symbol(
    name: &str,
    qname: &str,
    kind: SymbolKind,
    scope: Option<&str>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 10,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: scope.map(|s| s.to_string()),
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn make_ref(source_idx: usize, target: &str, kind: EdgeKind, line: u32) -> ExtractedRef {
    ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind,
        line,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "kotlin".to_string(),
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
        .map(|f| make_file(&f.path, f.symbols.clone(), f.refs.clone()))
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

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

#[test]
fn test_kotlin_extension_function_resolves_by_receiver() {
    // `s.shout()` binds to the top-level extension `fun String.shout()`. The
    // extension is keyed under the bare qname `shout`, not `String.shout`, so
    // it resolves via the generic extension-method fallback keyed on the
    // receiver type folded into the signature as `(this String`.
    let mut ext = make_symbol("shout", "shout", SymbolKind::Function, None);
    ext.signature = Some("fun shout(this String): String".to_string());

    let file = make_file("src/Ext.kt", vec![ext], vec![]);

    let (index, id_map) = build_test_env(&[&file]);
    let resolver = KotlinResolver;
    let file_ctx = resolver.build_file_context(&file, None);

    let mut chain = make_chain(&["s", "shout"]);
    chain.segments[0].declared_type = Some("String".to_string());
    let mut call_ref = make_ref(0, "shout", EdgeKind::Calls, 5);
    call_ref.chain = Some(chain);

    let ref_ctx = RefContext {
        extracted_ref: &call_ref,
        source_symbol: &file.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };

    let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(result.is_some(), "s.shout() should resolve to the extension function");
    let res = result.unwrap();
    assert_eq!(res.strategy, "chain_extension_method");
    assert_eq!(
        res.target_symbol_id,
        *id_map.get(&("src/Ext.kt".to_string(), "shout".to_string())).unwrap()
    );
}
