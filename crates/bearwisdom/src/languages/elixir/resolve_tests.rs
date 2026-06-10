use super::hooks::{
    detect_elixir_ecto_emission, detect_elixir_grpc_emission, detect_elixir_http_emission,
    detect_elixir_mailer_emission, detect_elixir_oban_emission, detect_elixir_phoenix_channel_use,
    ElixirHooks,
};
use super::profile::ELIXIR_PROFILE;
use crate::indexer::resolve::engine::{RefContext, SymbolIndex};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::*;
use std::collections::HashMap;

#[test]
fn test_elixir_ecto_repo_get_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_elixir_ecto_emission("Repo", "get").unwrap() {
        FlowEmission::DbQuery {
            entity_name,
            operation,
        } => {
            assert_eq!(entity_name, "ex.*");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_elixir_ecto_repo_insert_emits_db_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_elixir_ecto_emission("MyApp.Repo", "insert").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_elixir_ecto_rejects_non_repo() {
    assert!(detect_elixir_ecto_emission("Logger", "get").is_none());
}

#[test]
fn test_elixir_httpoison_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_elixir_http_emission("HTTPoison", "get", &args).unwrap() {
        FlowEmission::NamedChannel {
            kind,
            role,
            method,
            name,
            ..
        } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(method, Some(HttpMethod::Get));
            assert_eq!(name, "/api/users");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_elixir_tesla_post_emits_producer() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    let args = vec![
        CallArg::Ident("client".to_string()),
        CallArg::StringLit("/x".to_string()),
    ];
    assert!(matches!(
        detect_elixir_http_emission("Tesla", "post", &args).unwrap(),
        FlowEmission::NamedChannel { .. }
    ));
}

#[test]
fn test_elixir_http_rejects_non_http_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_elixir_http_emission("Logger", "get", &args).is_none());
}

#[test]
fn test_elixir_grpc_stub_emits_rpc_call() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    match detect_elixir_grpc_emission("Helloworld.Greeter.Stub", "say_hello").unwrap() {
        FlowEmission::NamedChannel {
            kind, role, name, ..
        } => {
            assert!(matches!(kind, NamedChannelKind::RpcCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "Greeter.say_hello");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_elixir_grpc_rejects_non_stub() {
    assert!(detect_elixir_grpc_emission("MyApp.Service", "call").is_none());
}

#[test]
fn test_elixir_oban_insert_emits_bg_job() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    match detect_elixir_oban_emission("Oban", "insert", &[]).unwrap() {
        FlowEmission::NamedChannel { kind, name, .. } => {
            assert!(matches!(kind, NamedChannelKind::BgJob));
            assert_eq!(name, "oban.job");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_elixir_bamboo_deliver_now_emits_mailer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    match detect_elixir_mailer_emission("MyApp.Mailer", "deliver_now").unwrap() {
        FlowEmission::NamedChannel {
            kind, role, name, ..
        } => {
            assert!(matches!(kind, NamedChannelKind::Mailer));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "ex.Mailer");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_elixir_swoosh_deliver_emits_mailer() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, NamedChannelKind};
    match detect_elixir_mailer_emission("MyApp.UserMailer", "deliver").unwrap() {
        FlowEmission::NamedChannel { kind, .. } => {
            assert!(matches!(kind, NamedChannelKind::Mailer));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_elixir_mailer_rejects_non_mailer_module() {
    assert!(detect_elixir_mailer_emission("Logger", "deliver").is_none());
}

#[test]
fn test_elixir_phoenix_channel_use_emits_ws_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    match detect_elixir_phoenix_channel_use("Phoenix.Channel", None).unwrap() {
        FlowEmission::NamedChannel {
            kind, role, name, ..
        } => {
            assert!(matches!(kind, NamedChannelKind::WebSocket));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "ex.Phoenix.Channel");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_elixir_phoenix_live_view_recognised() {
    assert!(detect_elixir_phoenix_channel_use("Phoenix.LiveView", None).is_some());
}

#[test]
fn test_elixir_phoenix_channel_rejects_non_phoenix_module() {
    assert!(detect_elixir_phoenix_channel_use("Logger", None).is_none());
    assert!(detect_elixir_phoenix_channel_use("Ecto.Schema", None).is_none());
}

// ---------------------------------------------------------------------------
// alias→module-qname binding through the generic ladder (ELIXIR_PROFILE)
// ---------------------------------------------------------------------------

fn make_sym(name: &str, qname: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 5,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn make_alias_import(target_local: &str, module: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target_local.to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        col: 0,
        module: Some(module.to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_call(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 2,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 2,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "elixir".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 10,
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

#[test]
fn alias_module_qname_binds_bare_alias_to_module_symbol() {
    // `alias MyApp.Foo` then a bare `Foo` call binds to the module symbol whose
    // qname IS the import's full path — through the generic ladder, gated by
    // ELIXIR_PROFILE.alias_module_qname.
    let foo_module = make_file(
        "lib/my_app/foo.ex",
        vec![make_sym("Foo", "MyApp.Foo", SymbolKind::Module)],
        vec![],
    );
    let caller = make_file(
        "lib/my_app/bar.ex",
        vec![make_sym("Bar", "MyApp.Bar", SymbolKind::Module)],
        vec![make_alias_import("Foo", "MyApp.Foo"), make_call("Foo")],
    );

    let mut id_map = HashMap::new();
    let mut next = 1i64;
    for pf in [&foo_module, &caller] {
        for s in &pf.symbols {
            id_map.insert((pf.path.clone(), s.qualified_name.clone()), next);
            next += 1;
        }
    }
    let owned: Vec<ParsedFile> = [&foo_module, &caller]
        .iter()
        .map(|f| make_file(&f.path, f.symbols.clone(), f.refs.clone()))
        .collect();
    let index = SymbolIndex::build(&owned, &id_map);

    let file_ctx = ElixirHooks.build_file_context(&caller, None).unwrap();
    let r = &caller.refs[1]; // the bare `Foo` call
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &caller.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };
    let res = DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: &index,
        kind_compatible: |_, _| true,
    }
    .resolve_all_with_profile(&ELIXIR_PROFILE)
    .expect("bare alias should bind to MyApp.Foo through the generic ladder");

    assert_eq!(res.strategy, "default_alias_module_qname");
    let expected = *id_map
        .get(&("lib/my_app/foo.ex".to_string(), "MyApp.Foo".to_string()))
        .unwrap();
    assert_eq!(res.target_symbol_id, expected);
}
