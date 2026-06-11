use super::hooks::{
    classify_dart_import_uri, detect_dart_drift_emission, detect_dart_grpc_emission,
    detect_dart_http_chain, detect_dart_shelf_route,
};
use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::indexer::project_context::ProjectContext;
use crate::types::*;

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
