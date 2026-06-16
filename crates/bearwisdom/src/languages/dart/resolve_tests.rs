use super::hooks::{
    classify_dart_import_uri, detect_dart_drift_emission, detect_dart_grpc_emission,
    detect_dart_http_chain, detect_dart_shelf_route, DartHooks,
};
use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, ImportEntry, RefContext, SymbolIndex};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::*;
use std::collections::HashMap;

/// A `ProjectContext` whose pubspec declares `name: <self_pkg>` — the own-name
/// signal `classify_dart_import_uri` consults to tell a `package:<self>/...`
/// URI from a third-party one.
fn ctx_with_pubspec(self_pkg: &str) -> ProjectContext {
    let mut manifest = ManifestData::default();
    manifest.package_names.push(self_pkg.to_string());
    let mut ctx = ProjectContext::default();
    ctx.manifests.insert(ManifestKind::Pubspec, manifest);
    ctx
}

fn make_chain(segments: &[&str]) -> MemberChain {
    MemberChain {
        segments: segments
            .iter()
            .enumerate()
            .map(|(i, name)| ChainSegment {
                name: name.to_string(),
                node_kind: "test".to_string(),
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
fn test_dart_shelf_route_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_dart_shelf_route("get", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_dart_shelf_post_works() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_dart_shelf_route("post", &args).is_some());
}

#[test]
fn test_dart_shelf_rejects_unknown() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_dart_shelf_route("middleware", &args).is_none());
}

#[test]
fn test_dart_dio_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    let chain = make_chain(&["dio", "get"]);
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_dart_http_chain(&chain, &args).unwrap() {
        FlowEmission::NamedChannel { role, .. } => assert_eq!(role, ChannelRole::Producer),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_dart_http_rejects_other_root() {
    let chain = make_chain(&["myObj", "get"]);
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_dart_http_chain(&chain, &args).is_none());
}

#[test]
fn test_dart_drift_select_emits_db_query() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["select", "get"]);
    match detect_dart_drift_emission(&chain).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_dart_drift_update_emits_update() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let chain = make_chain(&["update", "write"]);
    match detect_dart_drift_emission(&chain).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Update),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_dart_grpc_emits_rpc_call() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let chain = make_chain(&["UserServiceClient", "getUser"]);
    match detect_dart_grpc_emission(&chain).unwrap() {
        FlowEmission::NamedChannel {
            kind, role, name, ..
        } => {
            assert!(matches!(kind, NamedChannelKind::RpcCall));
            assert_eq!(role, ChannelRole::Producer);
            assert_eq!(name, "UserService.getUser");
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_dart_grpc_rejects_non_client() {
    let chain = make_chain(&["UserService", "getUser"]);
    assert!(detect_dart_grpc_emission(&chain).is_none());
}

#[test]
fn classify_package_uri_brands_external() {
    // The URI a library prefix routes through (`i0` → `package:drift/...`)
    // classifies to the package namespace with no manifest needed.
    assert_eq!(
        classify_dart_import_uri("package:drift/drift.dart", None, None).as_deref(),
        Some("drift")
    );
}

#[test]
fn classify_dart_scheme_brands_stdlib() {
    assert_eq!(
        classify_dart_import_uri("dart:async", None, None).as_deref(),
        Some("dart.stdlib")
    );
}

#[test]
fn classify_relative_uri_is_project_local() {
    // A relative import (`i2` → `tables.dart`) is project-local, so the
    // prefixed ref is bound in the index rather than branded external.
    assert!(classify_dart_import_uri("tables.dart", None, None).is_none());
    assert!(classify_dart_import_uri("../models/user.dart", None, None).is_none());
}

#[test]
fn classify_self_package_uri_is_project_local() {
    // `package:<self>/...` is the idiomatic intra-package absolute import.
    // With the pubspec `name:` matching the URI's package segment, the ref is
    // project-local — bound under `lib/`, not branded external.
    let ctx = ctx_with_pubspec("myapp");
    assert!(classify_dart_import_uri("package:myapp/models/user.dart", None, Some(&ctx)).is_none());
}

#[test]
fn classify_third_party_package_uri_stays_external() {
    // A `package:` URI whose segment is not the own package brands external by
    // its package name, even alongside the self-package guard.
    let ctx = ctx_with_pubspec("myapp");
    assert_eq!(
        classify_dart_import_uri("package:third_party/x.dart", None, Some(&ctx)).as_deref(),
        Some("third_party")
    );
}

#[test]
fn classify_dart_scheme_stays_external_with_self_package() {
    // The self-package guard never affects `dart:` URIs — stdlib stays external.
    let ctx = ctx_with_pubspec("myapp");
    assert_eq!(
        classify_dart_import_uri("dart:io", None, Some(&ctx)).as_deref(),
        Some("dart.stdlib")
    );
}

#[test]
fn classify_workspace_sibling_package_is_project_local() {
    // In a melos workspace the union pubspec carries every member's name, so a
    // sibling `package:` URI is recognized as project-local rather than
    // external.
    let mut ctx = ctx_with_pubspec("myapp");
    if let Some(m) = ctx.manifests.get_mut(&ManifestKind::Pubspec) {
        m.package_names.push("myapp_core".to_string());
    }
    assert!(classify_dart_import_uri("package:myapp_core/api.dart", None, Some(&ctx)).is_none());
}

// ---------------------------------------------------------------------------
// Cross-package import attribution — `resolve_bare_post`
// ---------------------------------------------------------------------------

fn dart_file(
    path: &str,
    package_id: Option<i64>,
    symbols: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "dart".to_string(),
        content_hash: "x".to_string(),
        size: 100,
        line_count: 10,
        mtime: None,
        package_id,
        symbols,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn class_sym(name: &str) -> ExtractedSymbol {
    typed_sym(name, SymbolKind::Class)
}

fn typed_sym(name: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
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

fn use_ref(target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind,
        line: 5,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

/// A `FileContext` whose only import is one `package:<pkg>/...` URI.
fn ctx_importing(file_path: &str, package_uri: &str) -> FileContext {
    FileContext {
        file_path: file_path.to_string(),
        language: "dart".to_string(),
        imports: vec![ImportEntry {
            imported_name: package_uri.to_string(),
            module_path: Some(package_uri.to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    }
}

/// A two-file two-package index: `lib_pkg` (id 22) declares the named classes;
/// `app_pkg` (id 1) is the importing file. The `ProjectContext` registers the
/// sibling package's declared name so `workspace_package_id` maps the import.
fn two_package_index(
    lib_decl: &str,
    lib_classes: &[ExtractedSymbol],
    extra_app_symbols: Vec<ExtractedSymbol>,
    next_id_start: i64,
) -> (ProjectContext, SymbolIndex) {
    let lib = dart_file(
        "packages/lib_pkg/lib/widget.dart",
        Some(22),
        lib_classes.to_vec(),
        vec![],
    );
    let app = dart_file(
        "lib/screen.dart",
        Some(1),
        extra_app_symbols,
        vec![],
    );

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    let mut id = next_id_start;
    for s in lib_classes {
        id_map.insert(("packages/lib_pkg/lib/widget.dart".to_string(), s.qualified_name.clone()), id);
        id += 1;
    }
    for s in &app.symbols {
        id_map.insert(("lib/screen.dart".to_string(), s.qualified_name.clone()), id);
        id += 1;
    }

    let mut ctx = ProjectContext::default();
    ctx.workspace_pkg_by_declared_name
        .insert(lib_decl.to_string(), 22);

    let index = SymbolIndex::build_with_context(&[lib, app], &id_map, Some(&ctx));
    (ctx, index)
}

#[test]
fn cross_package_import_binds_type_ref_and_instantiates() {
    // The app file imports `package:ui_kit/widget.dart` and uses the sibling's
    // `Spacer` class as a type and a constructor target. Both bind to the class
    // declared in the imported workspace package.
    let (_ctx, index) = two_package_index("ui_kit", &[class_sym("Spacer")], vec![], 100);
    let file_ctx = ctx_importing("lib/screen.dart", "package:ui_kit/widget.dart");
    let src = class_sym("Screen");

    for kind in [EdgeKind::TypeRef, EdgeKind::Instantiates] {
        let r = use_ref("Spacer", kind);
        let ref_ctx = RefContext {
            extracted_ref: &r,
            source_symbol: &src,
            scope_chain: vec![],
            file_package_id: Some(1),
        };
        let res = DartHooks
            .resolve_bare_post(&ref_ctx, &file_ctx, &index)
            .unwrap_or_else(|| panic!("{kind:?} Spacer should bind to the imported package class"));
        assert_eq!(res.target_symbol_id, 100, "{kind:?} should bind id=100");
        assert_eq!(res.strategy, "dart_workspace_package_import");
        assert_eq!(res.confidence, 1.0);
    }
}

#[test]
fn relative_import_within_package_is_not_hijacked() {
    // A `package:<self>/...` import resolves to the file's own package; the
    // cross-package rung must skip it so intra-package references stay on the
    // generic path. The own package declares `Spacer`, but the importing file
    // is in that same package (id 1).
    let lib = dart_file("lib/widget.dart", Some(1), vec![class_sym("Spacer")], vec![]);
    let app = dart_file("lib/screen.dart", Some(1), vec![class_sym("Screen")], vec![]);
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("lib/widget.dart".to_string(), "Spacer".to_string()), 200);
    id_map.insert(("lib/screen.dart".to_string(), "Screen".to_string()), 201);

    let mut ctx = ProjectContext::default();
    ctx.workspace_pkg_by_declared_name
        .insert("myapp".to_string(), 1);
    let index = SymbolIndex::build_with_context(&[lib, app], &id_map, Some(&ctx));

    let file_ctx = ctx_importing("lib/screen.dart", "package:myapp/widget.dart");
    let src = class_sym("Screen");
    let r = use_ref("Spacer", EdgeKind::TypeRef);
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &src,
        scope_chain: vec![],
        file_package_id: Some(1),
    };
    assert!(
        DartHooks.resolve_bare_post(&ref_ctx, &file_ctx, &index).is_none(),
        "own-package import must not bind through the cross-package rung"
    );
}

#[test]
fn same_name_different_kind_binds_class_via_package_scope() {
    // `Spacer` is a class in the imported package AND a property in the app
    // package. The import-scoped lookup finds the class in the sibling package
    // and ignores the same-named property, so a `TypeRef` binds to the class.
    let (_ctx, index) = two_package_index(
        "ui_kit",
        &[class_sym("Spacer")],
        vec![typed_sym("Spacer", SymbolKind::Property)],
        300,
    );
    let file_ctx = ctx_importing("lib/screen.dart", "package:ui_kit/widget.dart");
    let src = class_sym("Screen");
    let r = use_ref("Spacer", EdgeKind::TypeRef);
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &src,
        scope_chain: vec![],
        file_package_id: Some(1),
    };
    let res = DartHooks
        .resolve_bare_post(&ref_ctx, &file_ctx, &index)
        .expect("Spacer TypeRef should bind to the imported class, not the local property");
    // id 300 is the sibling-package class (registered first in two_package_index).
    assert_eq!(res.target_symbol_id, 300, "must bind the class in the imported package");
    assert_eq!(res.strategy, "dart_workspace_package_import");
}
