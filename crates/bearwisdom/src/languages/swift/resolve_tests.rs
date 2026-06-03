use super::hooks::{detect_swift_grdb_emission, detect_swift_grpc_emission, detect_swift_http_chain, detect_swift_vapor_route, SWIFT_HOOKS};
use crate::indexer::resolve::engine::{build_scope_chain, RefContext, SymbolIndex};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::*;
use std::collections::HashMap;

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

fn make_symbol(name: &str, qname: &str, kind: SymbolKind, scope: Option<&str>) -> ExtractedSymbol {
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

fn make_ref(source_idx: usize, target: &str, kind: EdgeKind, module: Option<&str>) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind,
        line: 1,
        col: 0,
        module: module.map(|s| s.to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "swift".to_string(),
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
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn build_index(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
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

/// `import ModuleA` scopes a bare `Foo` ref to ModuleA's `Foo`, even though a
/// same-named `Foo` lives in ModuleB. The bind flows through the generic
/// `default_imported_namespace` strategy reached from the Swift hook tail.
#[test]
fn swift_import_scopes_bare_name_bind() {
    let mod_a = make_file(
        "Sources/ModuleA/Foo.swift",
        vec![make_symbol("Foo", "ModuleA.Foo", SymbolKind::Class, Some("ModuleA"))],
        vec![],
    );
    let mod_b = make_file(
        "Sources/ModuleB/Foo.swift",
        vec![make_symbol("Foo", "ModuleB.Foo", SymbolKind::Class, Some("ModuleB"))],
        vec![],
    );
    let app = make_file(
        "Sources/App/main.swift",
        vec![make_symbol("App", "App", SymbolKind::Class, Some("App"))],
        vec![
            make_ref(0, "ModuleA", EdgeKind::Imports, Some("ModuleA")),
            make_ref(0, "Foo", EdgeKind::TypeRef, None),
        ],
    );

    let (index, id_map) = build_index(&[&mod_a, &mod_b, &app]);
    let file_ctx = SWIFT_HOOKS.build_file_context(&app, None).expect("file context");
    let ref_ctx = RefContext {
        extracted_ref: &app.refs[1],
        source_symbol: &app.symbols[0],
        scope_chain: build_scope_chain(app.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let res = SWIFT_HOOKS
        .resolve_ref(&file_ctx, &ref_ctx, &index)
        .expect("import-scoped bind resolves");
    assert_eq!(res.strategy, "default_imported_namespace");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("Sources/ModuleA/Foo.swift".to_string(), "ModuleA.Foo".to_string()))
            .unwrap()
    );
}

/// Without the `import ModuleA`, the bare `Foo` ref does NOT whole-program-grep
/// to either same-named `Foo`. Nothing scopes it, so it stays unresolved.
#[test]
fn swift_bare_name_without_import_does_not_grep() {
    let mod_a = make_file(
        "Sources/ModuleA/Foo.swift",
        vec![make_symbol("Foo", "ModuleA.Foo", SymbolKind::Class, Some("ModuleA"))],
        vec![],
    );
    let mod_b = make_file(
        "Sources/ModuleB/Foo.swift",
        vec![make_symbol("Foo", "ModuleB.Foo", SymbolKind::Class, Some("ModuleB"))],
        vec![],
    );
    let app = make_file(
        "Sources/App/main.swift",
        vec![make_symbol("App", "App", SymbolKind::Class, Some("App"))],
        vec![make_ref(0, "Foo", EdgeKind::TypeRef, None)],
    );

    let (index, _id_map) = build_index(&[&mod_a, &mod_b, &app]);
    let file_ctx = SWIFT_HOOKS.build_file_context(&app, None).expect("file context");
    let ref_ctx = RefContext {
        extracted_ref: &app.refs[0],
        source_symbol: &app.symbols[0],
        scope_chain: build_scope_chain(app.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    assert!(
        SWIFT_HOOKS.resolve_ref(&file_ctx, &ref_ctx, &index).is_none(),
        "bare Foo must not bind to a coincidental same-name symbol without an import"
    );
}
