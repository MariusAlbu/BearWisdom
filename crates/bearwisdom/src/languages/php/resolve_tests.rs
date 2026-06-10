// =============================================================================
// php/resolve_tests.rs — namespace normalization, external classification,
// file-context construction, and flow-emission detector tests. Resolution
// itself runs through the generic engine (see
// type_checker/core/{chain,default_resolver}_tests.rs).
// =============================================================================

use super::hooks::{build_file_context_inner, normalize_php_ns};
use crate::indexer::resolve::engine::{build_scope_chain, RefContext, SymbolIndex};
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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn make_use(source_idx: usize, alias: &str, fqn: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name: alias.to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        col: 0,
        module: Some(fqn.to_string()),
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}
fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "php".to_string(),
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

// ---------------------------------------------------------------------------
// Namespace normalization
// ---------------------------------------------------------------------------

#[test]
fn test_normalize_php_ns() {
    assert_eq!(normalize_php_ns("App\\Models\\User"), "App.Models.User");
    assert_eq!(normalize_php_ns("\\App\\Models\\User"), "App.Models.User");
    assert_eq!(normalize_php_ns("App.Models.User"), "App.Models.User");
    assert_eq!(normalize_php_ns("Foo"), "Foo");
}

// ---------------------------------------------------------------------------
// External classification (PhpHooks::classify_external)
// ---------------------------------------------------------------------------

#[test]
fn test_infer_framework_external() {
    let file = make_file(
        "app/Controllers/Foo.php",
        vec![make_symbol(
            "Foo",
            "App.Foo",
            SymbolKind::Class,
            Visibility::Public,
            Some("App"),
        )],
        vec![make_use(0, "Controller", "Illuminate\\Routing\\Controller")],
    );

    let file_ctx = build_file_context_inner(&file, None);
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let ns = {
        use crate::type_checker::profile::hooks::LanguageEngineHooks;
        let empty_lookup = SymbolIndex::build(&[], &HashMap::new());
        crate::languages::php::hooks::PhpHooks.classify_external(
            &ref_ctx,
            &file_ctx,
            None,
            &empty_lookup,
        )
    };
    assert!(
        ns.is_some(),
        "Illuminate import should be inferred as external"
    );
}

// ---------------------------------------------------------------------------
// build_file_context
// ---------------------------------------------------------------------------

#[test]
fn test_build_file_context_extracts_namespace() {
    let file = make_file(
        "app/Models/User.php",
        vec![
            make_symbol(
                "App.Models",
                "App.Models",
                SymbolKind::Namespace,
                Visibility::Public,
                None,
            ),
            make_symbol(
                "User",
                "App.Models.User",
                SymbolKind::Class,
                Visibility::Public,
                Some("App.Models"),
            ),
        ],
        vec![],
    );

    let ctx = build_file_context_inner(&file, None);
    assert_eq!(ctx.file_namespace, Some("App.Models".to_string()));
}

#[test]
fn test_build_file_context_normalizes_backslash() {
    let file = make_file(
        "app/Controllers/Foo.php",
        vec![make_symbol(
            "Foo",
            "App.Foo",
            SymbolKind::Class,
            Visibility::Public,
            Some("App"),
        )],
        // use App\Models\User;
        vec![make_use(0, "User", "App\\Models\\User")],
    );

    let ctx = build_file_context_inner(&file, None);
    assert_eq!(ctx.imports.len(), 1);
    // Module path should be normalized to dotted form.
    assert_eq!(
        ctx.imports[0].module_path.as_deref(),
        Some("App.Models.User")
    );
    assert_eq!(ctx.imports[0].imported_name, "User");
}

// ---------------------------------------------------------------------------
// DbQuery flow emission — Eloquent + Doctrine
// ---------------------------------------------------------------------------

fn make_static_chain(segments: &[&str]) -> MemberChain {
    // First segment is class (TypeAccess), rest are Property.
    MemberChain {
        segments: segments
            .iter()
            .enumerate()
            .map(|(i, name)| ChainSegment {
                name: name.to_string(),
                node_kind: if i == 0 {
                    "class".to_string()
                } else {
                    "static_call_expression".to_string()
                },
                kind: if i == 0 {
                    SegmentKind::TypeAccess
                } else {
                    SegmentKind::Property
                },
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            })
            .collect(),
    }
}

fn make_instance_chain(segments: &[&str]) -> MemberChain {
    // First segment is variable identifier, rest are Property.
    MemberChain {
        segments: segments
            .iter()
            .enumerate()
            .map(|(i, name)| ChainSegment {
                name: name.to_string(),
                node_kind: if i == 0 {
                    "variable_name".to_string()
                } else {
                    "member_call_expression".to_string()
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
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            })
            .collect(),
    }
}

#[test]
fn test_php_eloquent_where_emits_select() {
    use super::hooks::detect_php_db_query_emission;
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};

    let chain = make_static_chain(&["User", "where"]);
    match detect_php_db_query_emission(&chain, &[]).unwrap() {
        FlowEmission::DbQuery {
            entity_name,
            operation,
        } => {
            assert_eq!(entity_name, "php.User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_php_eloquent_find_emits_select() {
    use super::hooks::detect_php_db_query_emission;
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};

    let chain = make_static_chain(&["Post", "find"]);
    match detect_php_db_query_emission(&chain, &[]).unwrap() {
        FlowEmission::DbQuery {
            entity_name,
            operation,
        } => {
            assert_eq!(entity_name, "php.Post");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_php_eloquent_create_emits_insert() {
    use super::hooks::detect_php_db_query_emission;
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};

    let chain = make_static_chain(&["Article", "create"]);
    match detect_php_db_query_emission(&chain, &[]).unwrap() {
        FlowEmission::DbQuery {
            entity_name,
            operation,
        } => {
            assert_eq!(entity_name, "php.Article");
            assert_eq!(operation, DbQueryOp::Insert);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_php_eloquent_destroy_emits_delete() {
    use super::hooks::detect_php_db_query_emission;
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};

    let chain = make_static_chain(&["Comment", "destroy"]);
    match detect_php_db_query_emission(&chain, &[]).unwrap() {
        FlowEmission::DbQuery {
            entity_name,
            operation,
        } => {
            assert_eq!(entity_name, "php.Comment");
            assert_eq!(operation, DbQueryOp::Delete);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_php_eloquent_chained_first_emits_on_leaf() {
    use super::hooks::detect_php_db_query_emission;
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};

    // `User::where(...)->orderBy(...)->first()` — chain root TypeAccess(User),
    // intermediate Property segments, leaf is `first` (Select).
    let chain = make_static_chain(&["User", "where", "orderBy", "first"]);
    match detect_php_db_query_emission(&chain, &[]).unwrap() {
        FlowEmission::DbQuery {
            entity_name,
            operation,
        } => {
            assert_eq!(entity_name, "php.User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_php_eloquent_firstorcreate_emits_upsert() {
    use super::hooks::detect_php_db_query_emission;
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};

    let chain = make_static_chain(&["User", "firstOrCreate"]);
    match detect_php_db_query_emission(&chain, &[]).unwrap() {
        FlowEmission::DbQuery {
            entity_name,
            operation,
        } => {
            assert_eq!(entity_name, "php.User");
            assert_eq!(operation, DbQueryOp::Upsert);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_php_doctrine_em_find_with_class_ident_arg() {
    use super::hooks::detect_php_db_query_emission;
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;

    // `$em->find(User::class, $id)` — chain is em.find, entity from arg.
    let chain = make_instance_chain(&["em", "find"]);
    let args = vec![
        CallArg::Ident("User".to_string()),
        CallArg::Ident("id".to_string()),
    ];
    match detect_php_db_query_emission(&chain, &args).unwrap() {
        FlowEmission::DbQuery {
            entity_name,
            operation,
        } => {
            assert_eq!(entity_name, "php.User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_php_doctrine_repository_chain_emits() {
    use super::hooks::detect_php_db_query_emission;
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;

    // `$em->getRepository(User::class)->findBy(...)` — chain is em.getRepository.findBy.
    // The Ident("User") arg was attached to the `findBy` call's call_args via
    // the doctrine path; here we simulate the leaf call which has no args, but
    // an intermediate getRepository captured the entity. In practice we
    // currently see args on the leaf-call ref; the test here uses args carried
    // on the leaf ref (the cross-call propagation is out of scope for v1).
    let chain = make_instance_chain(&["em", "getRepository", "findBy"]);
    let args = vec![CallArg::Ident("User".to_string())];
    match detect_php_db_query_emission(&chain, &args).unwrap() {
        FlowEmission::DbQuery {
            entity_name,
            operation,
        } => {
            assert_eq!(entity_name, "php.User");
            assert_eq!(operation, DbQueryOp::Select);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_php_em_flush_emits_other() {
    use super::hooks::detect_php_db_query_emission;
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};

    let chain = make_instance_chain(&["em", "flush"]);
    match detect_php_db_query_emission(&chain, &[]).unwrap() {
        FlowEmission::DbQuery {
            entity_name,
            operation,
        } => {
            assert_eq!(entity_name, "php.*");
            assert_eq!(operation, DbQueryOp::Other);
        }
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_php_facade_static_call_not_emitted() {
    use super::hooks::detect_php_db_query_emission;

    // `Route::get(...)` — Route is a Laravel facade, not an Eloquent model.
    let chain = make_static_chain(&["Route", "get"]);
    assert!(detect_php_db_query_emission(&chain, &[]).is_none());

    // `Auth::user()` — Auth facade, not a model.
    let chain = make_static_chain(&["Auth", "user"]);
    assert!(detect_php_db_query_emission(&chain, &[]).is_none());
}

#[test]
fn test_php_lowercase_root_not_emitted() {
    use super::hooks::detect_php_db_query_emission;

    // `$user->where(...)` — chain root is an Identifier, not a class.
    let chain = make_instance_chain(&["user", "where"]);
    assert!(detect_php_db_query_emission(&chain, &[]).is_none());
}

#[test]
fn test_php_unknown_leaf_not_emitted() {
    use super::hooks::detect_php_db_query_emission;

    // `User::someRandomMethod()` — leaf isn't a known Eloquent op.
    let chain = make_static_chain(&["User", "logBackground"]);
    assert!(detect_php_db_query_emission(&chain, &[]).is_none());
}

// ---------------------------------------------------------------------------
// Symfony Route attribute → Consumer HttpCall
// ---------------------------------------------------------------------------

#[test]
fn test_php_symfony_route_attribute_emits_consumer_httpcall() {
    use super::hooks::detect_symfony_route_attribute_emission;
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};

    let em = detect_symfony_route_attribute_emission("Route", Some("/api/users")).unwrap();
    match em {
        FlowEmission::NamedChannel {
            kind, name, role, ..
        } => {
            assert!(matches!(kind, NamedChannelKind::HttpCall));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "/api/users");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_php_symfony_route_no_url_returns_none() {
    use super::hooks::detect_symfony_route_attribute_emission;

    // No URL arg.
    assert!(detect_symfony_route_attribute_emission("Route", None).is_none());
    // Empty URL.
    assert!(detect_symfony_route_attribute_emission("Route", Some("")).is_none());
    // Non-Route attribute.
    assert!(detect_symfony_route_attribute_emission("Get", Some("/x")).is_none());
    // Non-path URL (skip non-route refs that happen to land on a Route ident).
    assert!(detect_symfony_route_attribute_emission("Route", Some("api/users")).is_none());
}

#[test]
fn test_php_ratchet_message_component_emits_ws_consumer() {
    use super::hooks::detect_php_ratchet_emission;
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    match detect_php_ratchet_emission("Ratchet\\MessageComponentInterface").unwrap() {
        FlowEmission::NamedChannel {
            kind, role, name, ..
        } => {
            assert!(matches!(kind, NamedChannelKind::WebSocket));
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(name, "php.ratchet");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_php_ratchet_rejects_non_ws_interface() {
    use super::hooks::detect_php_ratchet_emission;
    assert!(detect_php_ratchet_emission("SomeInterface").is_none());
}
