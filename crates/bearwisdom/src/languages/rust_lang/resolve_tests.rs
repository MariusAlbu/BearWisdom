use super::hooks::build_file_context_inner;
use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{build_scope_chain, RefContext};
use crate::types::*;

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
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
            col: 0,
}
}

fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "rust".to_string(),
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

fn cargo_ctx_with(deps: &[&str]) -> ProjectContext {
    let mut ctx = ProjectContext::default();
    let mut cargo = ManifestData::default();
    for d in deps {
        cargo.dependencies.insert((*d).to_string());
    }
    ctx.manifests.insert(ManifestKind::Cargo, cargo);
    ctx
}

// ---------------------------------------------------------------------------
// External namespace inference: bare `crate::path` attribution via Cargo.toml
// ---------------------------------------------------------------------------

#[test]
fn bare_anyhow_path_attributed_to_anyhow_not_std() {
    // Pre-fix bug: `is_rust_builtin` returned true for any name whose first
    // `::` segment matched a hardcoded crate list ("anyhow", "tokio", ...).
    // The caller then unconditionally returned `Some("std")`, silently
    // misattributing every match. Now the manifest is consulted directly.
    let ctx = cargo_ctx_with(&["anyhow"]);
    let file = make_file(
        "src/lib.rs",
        vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
        vec![make_ref(0, "anyhow::anyhow", EdgeKind::Calls, 5)],
    );

    let file_ctx = build_file_context_inner(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup =
            crate::indexer::resolve::engine::SymbolIndex::build(&[], &std::collections::HashMap::new());
        crate::languages::rust_lang::hooks::RustHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    assert_eq!(ns.as_deref(), Some("anyhow"));
}

#[test]
fn bare_hyphenated_crate_normalized_to_underscore() {
    // Cargo.toml declares `serde-json` (hypothetically); source uses
    // `serde_json::json`. Attribution should still match.
    let ctx = cargo_ctx_with(&["serde-json"]);
    let file = make_file(
        "src/lib.rs",
        vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
        vec![make_ref(0, "serde_json::json", EdgeKind::Calls, 5)],
    );

    let file_ctx = build_file_context_inner(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup =
            crate::indexer::resolve::engine::SymbolIndex::build(&[], &std::collections::HashMap::new());
        crate::languages::rust_lang::hooks::RustHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    assert_eq!(ns.as_deref(), Some("serde_json"));
}

#[test]
fn bare_stdlib_path_still_routes_to_std() {
    let ctx = cargo_ctx_with(&[]);
    let file = make_file(
        "src/lib.rs",
        vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
        vec![make_ref(0, "std::collections::HashMap", EdgeKind::TypeRef, 5)],
    );

    let file_ctx = build_file_context_inner(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup =
            crate::indexer::resolve::engine::SymbolIndex::build(&[], &std::collections::HashMap::new());
        crate::languages::rust_lang::hooks::RustHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    assert_eq!(ns.as_deref(), Some("std"));
}

#[test]
fn bare_crate_path_internal_not_attributed_external() {
    let ctx = cargo_ctx_with(&["anyhow"]);
    let file = make_file(
        "src/lib.rs",
        vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
        vec![make_ref(0, "crate::models::User", EdgeKind::TypeRef, 5)],
    );

    let file_ctx = build_file_context_inner(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup =
            crate::indexer::resolve::engine::SymbolIndex::build(&[], &std::collections::HashMap::new());
        crate::languages::rust_lang::hooks::RustHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    assert!(ns.is_none(), "crate:: paths should not be classified external");
}

#[test]
fn unknown_bare_path_not_attributed_when_not_in_manifest() {
    // A name that used to be in the hardcoded list but isn't in the manifest
    // should NOT be silently classified as external. Without the list, we
    // require the manifest entry to claim attribution.
    let ctx = cargo_ctx_with(&["serde"]); // anyhow NOT declared
    let file = make_file(
        "src/lib.rs",
        vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
        vec![make_ref(0, "anyhow::anyhow", EdgeKind::Calls, 5)],
    );

    let file_ctx = build_file_context_inner(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup =
            crate::indexer::resolve::engine::SymbolIndex::build(&[], &std::collections::HashMap::new());
        crate::languages::rust_lang::hooks::RustHooks.classify_external(
            &ref_ctx, &file_ctx, Some(&ctx), &empty_lookup,
        )
    };
    assert!(
        ns.is_none(),
        "anyhow::* without a manifest declaration must not auto-classify"
    );
}

// ---------------------------------------------------------------------------
// Flow emission detectors (Goal 18 — Rust backend frameworks)
// ---------------------------------------------------------------------------

fn make_chain(segments: &[(&str, SegmentKind)]) -> MemberChain {
    MemberChain {
        segments: segments
            .iter()
            .map(|(name, kind)| ChainSegment {
                name: (*name).to_string(),
                node_kind: "test".to_string(),
                kind: *kind,
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

// --- Axum -----------------------------------------------------------------

#[test]
fn test_rust_axum_router_route_emits_consumer_http() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_rust_axum_route_emission;

    let chain = make_chain(&[
        ("Router", SegmentKind::Identifier),
        ("new", SegmentKind::Property),
        ("route", SegmentKind::Property),
    ]);
    let args = vec![CallArg::StringLit("/api/users".to_string()), CallArg::Other];
    match detect_rust_axum_route_emission(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { kind, name, role, .. } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "/api/users");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_rust_axum_route_reads_verb_from_second_arg() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
    use super::hooks::detect_rust_axum_route_emission;

    let chain = make_chain(&[
        ("Router", SegmentKind::Identifier),
        ("new", SegmentKind::Property),
        ("route", SegmentKind::Property),
    ]);
    // `Router::new().route("/x", post(handler))` — Ident("post") in call_args[1].
    let args = vec![
        CallArg::StringLit("/api/users".to_string()),
        CallArg::Ident("post".to_string()),
    ];
    match detect_rust_axum_route_emission(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { method, .. } => assert_eq!(method, Some(HttpMethod::Post)),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_rust_axum_nest_emits_consumer_http() {
    use super::hooks::detect_rust_axum_route_emission;

    let chain = make_chain(&[
        ("Router", SegmentKind::Identifier),
        ("new", SegmentKind::Property),
        ("nest", SegmentKind::Property),
    ]);
    let args = vec![CallArg::StringLit("/api".to_string()), CallArg::Other];
    assert!(detect_rust_axum_route_emission(&chain, &args).is_some());
}

#[test]
fn test_rust_axum_route_rejects_non_router_root() {
    use super::hooks::detect_rust_axum_route_emission;

    let chain = make_chain(&[
        ("client", SegmentKind::Identifier),
        ("route", SegmentKind::Property),
    ]);
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_rust_axum_route_emission(&chain, &args).is_none());
}

// --- Actix-web / Rocket attribute -----------------------------------------

#[test]
fn test_rust_route_attribute_emits_consumer_http() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    use super::hooks::detect_rust_route_attribute_emission;

    let em = detect_rust_route_attribute_emission("get", Some("/api/users")).unwrap();
    match em {
        FlowEmission::NamedChannel { kind, name, role, method, .. } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "/api/users");
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_rust_route_attribute_rejects_non_url_arg() {
    use super::hooks::detect_rust_route_attribute_emission;

    assert!(detect_rust_route_attribute_emission("get", None).is_none());
    assert!(detect_rust_route_attribute_emission("get", Some("")).is_none());
    assert!(detect_rust_route_attribute_emission("get", Some("not-a-path")).is_none());
    assert!(detect_rust_route_attribute_emission("not_a_verb", Some("/x")).is_none());
}

#[test]
fn test_rust_actix_web_resource_emits_consumer() {
    use super::hooks::detect_rust_actix_resource_emission;

    let chain = make_chain(&[
        ("web", SegmentKind::Identifier),
        ("resource", SegmentKind::Property),
    ]);
    let args = vec![CallArg::StringLit("/users".to_string())];
    assert!(detect_rust_actix_resource_emission(&chain, &args).is_some());
}

// --- reqwest (Producer) ---------------------------------------------------

#[test]
fn test_rust_reqwest_client_get_emits_producer_http() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
    use super::hooks::detect_rust_reqwest_emission;

    let chain = make_chain(&[
        ("client", SegmentKind::Identifier),
        ("get", SegmentKind::Property),
    ]);
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_rust_reqwest_emission(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { kind, name, role, method, .. } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "/api/users");
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_rust_reqwest_post_with_absolute_url() {
    use crate::indexer::resolve::flow_emit::HttpMethod;
    use super::hooks::detect_rust_reqwest_emission;

    let chain = make_chain(&[
        ("client", SegmentKind::Identifier),
        ("post", SegmentKind::Property),
    ]);
    let args = vec![CallArg::StringLit("https://api.example.com/items".to_string())];
    let em = detect_rust_reqwest_emission(&chain, &args).unwrap();
    if let crate::indexer::resolve::flow_emit::FlowEmission::NamedChannel { method, .. } = em {
        assert_eq!(method, Some(HttpMethod::Post));
    } else {
        panic!("expected NamedChannel");
    }
}

#[test]
fn test_rust_reqwest_rejects_router_root() {
    use super::hooks::detect_rust_reqwest_emission;

    // `Router::new().get(...)` — Router prefix is reserved for axum.
    let chain = make_chain(&[
        ("Router", SegmentKind::Identifier),
        ("new", SegmentKind::Property),
        ("get", SegmentKind::Property),
    ]);
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_rust_reqwest_emission(&chain, &args).is_none());
}

// --- SQLx -----------------------------------------------------------------

#[test]
fn test_rust_sqlx_query_select_emits_db_query() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_rust_sqlx_macro_emission;

    let args = vec![CallArg::StringLit("SELECT * FROM users WHERE id = $1".to_string())];
    match detect_rust_sqlx_macro_emission("query", Some("sqlx"), &args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "rs.users");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_rust_sqlx_query_as_uses_entity_arg() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_rust_sqlx_macro_emission;

    // `sqlx::query_as!(User, "SELECT * FROM users")`.
    let args = vec![
        CallArg::Ident("User".to_string()),
        CallArg::StringLit("SELECT * FROM users".to_string()),
    ];
    match detect_rust_sqlx_macro_emission("query_as", Some("sqlx"), &args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "rs.User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_rust_sqlx_query_insert_op() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_rust_sqlx_macro_emission;

    let args = vec![CallArg::StringLit("INSERT INTO items (a) VALUES ($1)".to_string())];
    match detect_rust_sqlx_macro_emission("query", Some("sqlx"), &args).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "rs.items");
            assert_eq!(operation, DbQueryOp::Insert);
        }
        _ => panic!("expected DbQuery"),
    }
}

// --- Diesel ---------------------------------------------------------------

#[test]
fn test_rust_diesel_table_first_emits_db_query() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use super::hooks::detect_rust_diesel_emission;

    // `users::table.filter(...).first(&conn)`.
    let chain = make_chain(&[
        ("users", SegmentKind::Identifier),
        ("table", SegmentKind::Property),
        ("filter", SegmentKind::Property),
        ("first", SegmentKind::Property),
    ]);
    match detect_rust_diesel_emission(&chain).unwrap() {
        FlowEmission::DbQuery { entity_name, operation } => {
            assert_eq!(entity_name, "rs.users");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_rust_diesel_load_emits_db_query() {
    use super::hooks::detect_rust_diesel_emission;

    let chain = make_chain(&[
        ("posts", SegmentKind::Identifier),
        ("table", SegmentKind::Property),
        ("load", SegmentKind::Property),
    ]);
    assert!(detect_rust_diesel_emission(&chain).is_some());
}

#[test]
fn test_rust_diesel_no_table_segment_returns_none() {
    use super::hooks::detect_rust_diesel_emission;

    // No `table` segment — not Diesel.
    let chain = make_chain(&[
        ("foo", SegmentKind::Identifier),
        ("bar", SegmentKind::Property),
        ("first", SegmentKind::Property),
    ]);
    assert!(detect_rust_diesel_emission(&chain).is_none());
}

// --- Tonic ----------------------------------------------------------------

#[test]
fn test_rust_tonic_client_method_emits_rpc_call() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_rust_tonic_emission;

    let chain = make_chain(&[
        ("HelloServiceClient", SegmentKind::Identifier),
        ("new", SegmentKind::Property),
        ("say_hello", SegmentKind::Property),
    ]);
    match detect_rust_tonic_emission(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, name, role, .. } => {
            assert!(matches!(kind, NamedChannelKind::RpcCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "HelloService.say_hello");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_rust_tonic_connect_constructor_works() {
    use super::hooks::detect_rust_tonic_emission;

    let chain = make_chain(&[
        ("UserServiceClient", SegmentKind::Identifier),
        ("connect", SegmentKind::Property),
        ("get_user", SegmentKind::Property),
    ]);
    assert!(detect_rust_tonic_emission(&chain).is_some());
}

#[test]
fn test_rust_tonic_rejects_non_client_root() {
    use super::hooks::detect_rust_tonic_emission;

    // Root doesn't end in "Client".
    let chain = make_chain(&[
        ("HelloService", SegmentKind::Identifier),
        ("new", SegmentKind::Property),
        ("method", SegmentKind::Property),
    ]);
    assert!(detect_rust_tonic_emission(&chain).is_none());
}

// ---------------------------------------------------------------------------
// Goal 57 — Client type propagation through let-bindings
// ---------------------------------------------------------------------------

#[test]
fn test_rust_tonic_direct_detector_still_rejects_bare_variable() {
    use super::hooks::detect_rust_tonic_emission;
    // Direct detector still can't recover the type — only the
    // lookup-aware `detect_flow_emission_with_lookup` path does.
    let chain = make_chain(&[
        ("c", SegmentKind::Identifier),
        ("method", SegmentKind::Property),
    ]);
    assert!(detect_rust_tonic_emission(&chain).is_none());
}

#[test]
fn test_rust_tonic_let_bound_client_emits_via_lookup() {
    use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolInfo, SymbolLookup};
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use crate::types::{ChainSegment, ExtractedRef, ExtractedSymbol, MemberChain};

    struct VarLookup;
    impl SymbolLookup for VarLookup {
        fn by_name(&self, _: &str) -> &[SymbolInfo] { &[] }
        fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> { None }
        fn members_of(&self, _: &str) -> &[SymbolInfo] { &[] }
        fn types_by_name(&self, _: &str) -> &[SymbolInfo] { &[] }
        fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> { Vec::new() }
        fn has_in_namespace(&self, _: &str) -> bool { false }
        fn in_file(&self, _: &str) -> &[SymbolInfo] { &[] }
        fn field_type_name(&self, qname: &str) -> Option<&str> {
            if qname == "main.c" { Some("HelloServiceClient") } else { None }
        }
        fn return_type_name(&self, _: &str) -> Option<&str> { None }
        fn field_type_args(&self, _: &str) -> Option<&[String]> { None }
        fn generic_params(&self, _: &str) -> Option<&[String]> { None }
        fn reexports_from(&self, _: &str) -> &[(String, String)] { &[] }
        fn is_external_name(&self, _: &str, _: &str) -> bool { false }
    }

    let chain = MemberChain {
        segments: vec![
            ChainSegment { name: "c".to_string(), node_kind: "identifier".to_string(), kind: SegmentKind::Identifier, declared_type: None, type_args: vec![], optional_chaining: false, byte_offset: 0, declared_type_id: None, is_call: false, type_arg_ids: Vec::new() },
            ChainSegment { name: "say_hello".to_string(), node_kind: "field_expression".to_string(), kind: SegmentKind::Property, declared_type: None, type_args: vec![], optional_chaining: false, byte_offset: 0, declared_type_id: None, is_call: false, type_arg_ids: Vec::new() },
        ],
    };
    let extracted_ref = ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: 0,
        target_name: "say_hello".to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        col: 0,
        module: None,
        chain: Some(chain),
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };
    let source_symbol = ExtractedSymbol {
        name: "main".to_string(),
        qualified_name: "main".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 1, end_line: 1, start_col: 0, end_col: 0,
        signature: None, doc_comment: None,
        scope_path: Some("main".to_string()),
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
};
    let ref_ctx = RefContext {
        extracted_ref: &extracted_ref,
        source_symbol: &source_symbol,
        scope_chain: vec!["main".to_string()],
        file_package_id: None,
    };
    let file_ctx = FileContext {
        file_path: "main.rs".to_string(),
        language: "rust".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let emissions = super::hooks::detect_flow_inner_with_lookup(&file_ctx, &ref_ctx, &VarLookup);
    assert!(
        matches!(emissions.first(), Some(FlowEmission::NamedChannel { kind: NamedChannelKind::RpcCall, .. })),
        "expected RpcCall via let-binding propagation, got {emissions:?}"
    );
}

// ---------------------------------------------------------------------------
// Goal 75 — false-positive tightening
// ---------------------------------------------------------------------------

#[test]
fn test_rust_axum_route_no_emit_for_relative_url() {
    use super::hooks::detect_rust_axum_route_emission;
    let chain = make_chain(&[
        ("Router", SegmentKind::Identifier),
        ("new", SegmentKind::Property),
        ("route", SegmentKind::Property),
    ]);
    let args = vec![CallArg::StringLit("api/users".to_string())];
    assert!(detect_rust_axum_route_emission(&chain, &args).is_none());
}

#[test]
fn test_rust_diesel_no_emit_without_table_segment() {
    use super::hooks::detect_rust_diesel_emission;
    let chain = make_chain(&[
        ("users", SegmentKind::Identifier),
        ("filter", SegmentKind::Property),
        ("first", SegmentKind::Property),
    ]);
    assert!(detect_rust_diesel_emission(&chain).is_none());
}

#[test]
fn test_rust_reqwest_no_emit_for_server_root() {
    use super::hooks::detect_rust_reqwest_emission;
    let chain = make_chain(&[
        ("Server", SegmentKind::Identifier),
        ("new", SegmentKind::Property),
        ("get", SegmentKind::Property),
    ]);
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_rust_reqwest_emission(&chain, &args).is_none());
}

// --- Apalis / rdkafka / Redis / UDS ---------------------------------------

#[test]
fn test_rust_apalis_storage_push_emits_bgjob() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_rust_apalis_bgjob;
    let chain = make_chain(&[
        ("MemoryStorage", SegmentKind::Identifier),
        ("push", SegmentKind::Property),
    ]);
    match detect_rust_apalis_bgjob(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::BgJob));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "rs.apalis");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_rust_rdkafka_producer_send_emits_mq() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_rust_rdkafka_mq;
    let chain = make_chain(&[
        ("producer", SegmentKind::Identifier),
        ("send", SegmentKind::Property),
    ]);
    match detect_rust_rdkafka_mq(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::MessageQueue));
            assert_eq!(name, "rs.kafka");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_rust_lapin_basic_publish_emits_mq() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    use super::hooks::detect_rust_rdkafka_mq;
    let chain = make_chain(&[
        ("channel", SegmentKind::Identifier),
        ("basic_publish", SegmentKind::Property),
    ]);
    match detect_rust_rdkafka_mq(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::MessageQueue));
            assert_eq!(name, "rs.amqp");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_rust_redis_get_emits_config_lookup() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_rust_redis_config_lookup;
    let chain = make_chain(&[
        ("con", SegmentKind::Identifier),
        ("get", SegmentKind::Property),
    ]);
    let args = vec![CallArg::StringLit("session:abc".to_string())];
    match detect_rust_redis_config_lookup(&chain, &args).unwrap() {
        FlowEmission::ConfigLookup { key } => assert_eq!(key, "redis:session:abc"),
        _ => panic!("expected ConfigLookup"),
    }
}

#[test]
fn test_rust_uds_listener_bind_emits_ipc_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_rust_uds_emission;
    let chain = make_chain(&[
        ("UnixListener", SegmentKind::Identifier),
        ("bind", SegmentKind::Property),
    ]);
    let args = vec![CallArg::StringLit("/tmp/app.sock".to_string())];
    match detect_rust_uds_emission(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::IpcCall));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "/tmp/app.sock");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_rust_uds_stream_connect_emits_ipc_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    use super::hooks::detect_rust_uds_emission;
    let chain = make_chain(&[
        ("UnixStream", SegmentKind::Identifier),
        ("connect", SegmentKind::Property),
    ]);
    let args = vec![CallArg::StringLit("/tmp/app.sock".to_string())];
    match detect_rust_uds_emission(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { role, .. } => assert_eq!(role, ChannelRole::Producer),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_rust_axum_ws_on_upgrade_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_rust_axum_ws_consumer;
    let chain = make_chain(&[
        ("ws", SegmentKind::Identifier),
        ("on_upgrade", SegmentKind::Property),
    ]);
    match detect_rust_axum_ws_consumer(&chain).unwrap() {
        FlowEmission::NamedChannel { kind, role, .. } => {
            assert!(matches!(kind, NamedChannelKind::WebSocket));
            assert_eq!(role, ChannelRole::Consumer);
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_rust_async_graphql_object_emits_graphql_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use super::hooks::detect_rust_async_graphql_attribute;
    match detect_rust_async_graphql_attribute("Object").unwrap() {
        FlowEmission::NamedChannel { kind, role, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::GraphQLOp));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "rs.graphql.query");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_rust_async_graphql_subscription_emits_subscription() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use super::hooks::detect_rust_async_graphql_attribute;
    match detect_rust_async_graphql_attribute("Subscription").unwrap() {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "rs.graphql.subscription"),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_rust_juniper_graphql_object_recognised() {
    use super::hooks::detect_rust_async_graphql_attribute;
    assert!(detect_rust_async_graphql_attribute("graphql_object").is_some());
}

#[test]
fn test_rust_async_graphql_rejects_unrelated() {
    use super::hooks::detect_rust_async_graphql_attribute;
    assert!(detect_rust_async_graphql_attribute("derive").is_none());
    assert!(detect_rust_async_graphql_attribute("test").is_none());
}

// ---------------------------------------------------------------------------
// Step 7 — Rust prelude resolution against indexed `rust-stdlib` externals.
// ---------------------------------------------------------------------------

mod prelude {
    use super::*;
    use crate::indexer::resolve::engine::SymbolIndex;
    use std::collections::HashMap;

    fn stdlib_path(rel: &str) -> String {
        format!(
            "ext:idx:C:/toolchain/lib/rustlib/src/rust/library/{rel}"
        )
    }

    fn external_file(path: &str, symbols: Vec<ExtractedSymbol>) -> ParsedFile {
        ParsedFile {
            path: path.to_string(),
            language: "rust".to_string(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            content: None,
            has_errors: false,
            symbols,
            refs: vec![],
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

    /// Build a SymbolIndex from the given files, faking the symbol_id_map
    /// the indexer normally produces.
    fn build_index(files: &[ParsedFile]) -> SymbolIndex {
        let mut id_map: HashMap<(String, String), i64> = HashMap::new();
        let mut next: i64 = 1;
        for pf in files {
            for sym in &pf.symbols {
                id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next);
                next += 1;
            }
        }
        SymbolIndex::build(files, &id_map)
    }

    /// Drive a chain-less bare-name ref through the generic engine ladder
    /// against a real `SymbolIndex` (the production lookup whose
    /// `is_ambient_path` recognizes the `rust-stdlib` source subtree). A bare
    /// prelude name binds via the ladder's ambient-package strategy —
    /// `default_ambient_package` — exactly as it does in the resolve loop.
    fn run_resolve(
        ctx: &ProjectContext,
        caller_idx: usize,
        files: &[ParsedFile],
    ) -> Option<crate::indexer::resolve::engine::Resolution> {
        use crate::type_checker::core::DefaultResolver;
        let caller = &files[caller_idx];
        let index = build_index(files);
        let file_ctx = build_file_context_inner(caller, Some(ctx));
        let ref_ctx = RefContext {
            extracted_ref: &caller.refs[0],
            source_symbol: &caller.symbols[0],
            scope_chain: build_scope_chain(caller.symbols[0].scope_path.as_deref()),
            file_package_id: None,
        };
        DefaultResolver {
            file_ctx: &file_ctx,
            ref_ctx: &ref_ctx,
            lookup: &index,
            kind_compatible: |_, _| true,
        }
        .resolve_all_with_profile(&super::super::profile::RUST_PROFILE)
    }

    #[test]
    fn prelude_bare_vec_typeref_resolves_via_ambient_package() {
        // A bare `Vec` TypeRef with no internal collision binds through the
        // generic ambient-package strategy: the stdlib source subtree is an
        // ambient path (`ecosystem/ambient.rs`), so the lone kind-compatible
        // ambient candidate is the answer.
        let files = vec![
            external_file(
                &stdlib_path("alloc/src/vec/mod.rs"),
                vec![make_symbol("Vec", "Vec", SymbolKind::Struct, Visibility::Public, None)],
            ),
            make_file(
                "src/lib.rs",
                vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
                vec![make_ref(0, "Vec", EdgeKind::TypeRef, 5)],
            ),
        ];
        let res = run_resolve(&cargo_ctx_with(&[]), 1, &files)
            .expect("Vec must resolve");
        assert_eq!(res.strategy, "default_ambient_package");
    }

    #[test]
    fn prelude_vec_prefers_stdlib_over_internal_enum_variant() {
        let files = vec![
            // Internal collision: `<Enum>.Vec` variant shares the bare name.
            // It is NOT on an ambient path, so the ambient-package strategy
            // filters it out and binds the stdlib struct.
            make_file(
                "src/percentile.rs",
                vec![
                    make_symbol(
                        "PercentileValues",
                        "PercentileValues",
                        SymbolKind::Enum,
                        Visibility::Public,
                        None,
                    ),
                    make_symbol(
                        "Vec",
                        "PercentileValues.Vec",
                        SymbolKind::EnumMember,
                        Visibility::Public,
                        Some("PercentileValues"),
                    ),
                ],
                vec![],
            ),
            external_file(
                &stdlib_path("alloc/src/vec/mod.rs"),
                vec![make_symbol("Vec", "Vec", SymbolKind::Struct, Visibility::Public, None)],
            ),
            make_file(
                "src/lib.rs",
                vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
                vec![make_ref(0, "Vec", EdgeKind::TypeRef, 5)],
            ),
        ];
        let res = run_resolve(&cargo_ctx_with(&[]), 2, &files)
            .expect("Vec should still resolve, preferring stdlib");
        assert_eq!(res.strategy, "default_ambient_package");
    }

    #[test]
    fn prelude_some_resolves_to_option_variant() {
        // `Some(x)` is a Calls ref against the `Option.Some` enum_member on the
        // ambient stdlib path. The profile's kind table accepts EnumMember for a
        // Calls edge (tuple-variant construction), so the ambient-package
        // strategy binds it.
        let files = vec![
            external_file(
                &stdlib_path("core/src/option.rs"),
                vec![
                    make_symbol("Option", "Option", SymbolKind::Enum, Visibility::Public, None),
                    make_symbol(
                        "Some",
                        "Option.Some",
                        SymbolKind::EnumMember,
                        Visibility::Public,
                        Some("Option"),
                    ),
                ],
            ),
            make_file(
                "src/lib.rs",
                vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
                vec![make_ref(0, "Some", EdgeKind::Calls, 5)],
            ),
        ];
        let res = run_resolve(&cargo_ctx_with(&[]), 1, &files)
            .expect("Some should resolve to Option.Some");
        assert_eq!(res.strategy, "default_ambient_package");
        assert_eq!(res.target_symbol_id, 2);
    }

    #[test]
    fn ambient_package_resolves_any_stdlib_symbol_not_a_prelude_list() {
        // No hardcoded prelude name list gates the bind: ANY kind-compatible
        // symbol on the stdlib ambient path binds by bare name. `MyType` is not
        // a prelude name yet still resolves — the gate is the ambient PATH, not a
        // hand-maintained name set.
        let files = vec![
            external_file(
                &stdlib_path("alloc/src/vec/mod.rs"),
                vec![make_symbol("MyType", "MyType", SymbolKind::Struct, Visibility::Public, None)],
            ),
            make_file(
                "src/lib.rs",
                vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
                vec![make_ref(0, "MyType", EdgeKind::TypeRef, 5)],
            ),
        ];
        let res = run_resolve(&cargo_ctx_with(&[]), 1, &files)
            .expect("an ambient stdlib symbol binds by bare name");
        assert_eq!(res.strategy, "default_ambient_package");
    }

    #[test]
    fn third_party_crate_with_same_name_is_not_ambient() {
        // A cargo-registry crate sharing a prelude name lives under
        // `/registry/src/`, NOT the ambient stdlib subtree, so the
        // ambient-package strategy declines and (with no import) the bare name
        // stays unbound for the engine — to be branded external afterward.
        let files = vec![
            external_file(
                "ext:idx:C:/Users/Reaper/.cargo/registry/src/index.crates.io-x/bumpalo-3.20.2/src/collections/vec.rs",
                vec![make_symbol("Vec", "Vec", SymbolKind::Struct, Visibility::Public, None)],
            ),
            make_file(
                "src/lib.rs",
                vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
                vec![make_ref(0, "Vec", EdgeKind::TypeRef, 5)],
            ),
        ];
        let res = run_resolve(&cargo_ctx_with(&[]), 1, &files);
        if let Some(r) = res {
            assert_ne!(r.strategy, "default_ambient_package");
        }
    }
}
