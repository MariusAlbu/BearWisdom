use super::hooks::{detect_swift_grdb_emission, detect_swift_grpc_emission, detect_swift_http_chain, detect_swift_vapor_route};
use crate::types::*;

use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, SymbolInfo, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// SymbolLookup fixture for the Swift explicit-member-import strategy.
//
// Owns one `by_name` map; everything else is empty. The strategy under test
// only consults `by_name` + `is_external_file` (default `ext:` prefix).
// ---------------------------------------------------------------------------

struct ByNameFixture {
    by_name_map: HashMap<String, Vec<SymbolInfo>>,
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl ByNameFixture {
    fn with(name: &str, syms: Vec<SymbolInfo>) -> Self {
        let mut by_name_map: HashMap<String, Vec<SymbolInfo>> = HashMap::new();
        by_name_map.insert(name.to_string(), syms);
        Self {
            by_name_map,
            empty: Vec::new(),
            empty_reexports: Vec::new(),
        }
    }
}

/// Build a `RefContext` for a bare ref with NO module and NO chain — the
/// no-import same-module case the module-scope rung targets. The borrowed
/// `extracted`/`source` outlive the returned context.
fn module_scope_ctx<'a>(
    extracted: &'a ExtractedRef,
    source: &'a ExtractedSymbol,
) -> RefContext<'a> {
    RefContext {
        extracted_ref: extracted,
        source_symbol: source,
        scope_chain: Vec::new(),
        file_package_id: None,
    }
}

fn swift_file_ctx(path: &str) -> FileContext {
    FileContext {
        file_path: path.to_string(),
        language: "swift".to_string(),
        imports: vec![],
        file_namespace: None,
    }
}

/// A bare `User` TypeRef from `Sources/App/Handlers/Sub/Use.swift` with NO
/// import binds the unique internal struct `User` declared in a DIFFERENT
/// nested dir of the same `Sources/App/` target subtree — the whole-module
/// no-import case. `resolve_via_same_dir` cannot do this (different immediate
/// parent dir); the `SourcesTargetSubtree` boundary spans the whole target.
#[test]
fn swift_same_module_subtree_binds_unique_internal_type() {
    let user = make_resolve_sym(
        91,
        "User",
        "User",
        "struct",
        "Sources/App/Models/User.swift",
    );
    let fix = ByNameFixture::with("User", vec![user]);
    let file_ctx = swift_file_ctx("Sources/App/Handlers/Sub/Use.swift");
    let source_sym = make_resolve_source("use", "use");
    let extracted = make_typeref("User");
    let ref_ctx = module_scope_ctx(&extracted, &source_sym);

    let res = (DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: &fix,
        kind_compatible: super::predicates::kind_compatible,
    })
    .resolve_all_with_profile(&super::profile::SWIFT_PROFILE)
    .expect("same-module subtree binds the unique internal struct");
    assert_eq!(res.strategy, "default_module_scope");
    assert_eq!(res.target_symbol_id, 91);
}

/// Soundness: the only `User` candidate lives under `Sources/Other/` — a
/// DIFFERENT target subtree, a different module. No import brings it in, so
/// the module-scope rung must DECLINE (distinct subtree prefixes).
#[test]
fn swift_same_module_declines_cross_target() {
    let user = make_resolve_sym(
        92,
        "User",
        "User",
        "struct",
        "Sources/Other/User.swift",
    );
    let fix = ByNameFixture::with("User", vec![user]);
    let file_ctx = swift_file_ctx("Sources/App/Use.swift");
    let source_sym = make_resolve_source("use", "use");
    let extracted = make_typeref("User");
    let ref_ctx = module_scope_ctx(&extracted, &source_sym);

    let res = (DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: &fix,
        kind_compatible: super::predicates::kind_compatible,
    })
    .resolve_all_with_profile(&super::profile::SWIFT_PROFILE);
    let strategy = res.map(|r| r.strategy).unwrap_or("");
    assert_ne!(
        strategy, "default_module_scope",
        "a candidate in a different Sources/<Target>/ subtree is a different module"
    );
}

/// Soundness: two internal `User` structs both under `Sources/App/**` →
/// unique-internal-name dedup yields >1 → DECLINE (no coincidental guess).
#[test]
fn swift_same_module_declines_when_two_candidates() {
    let a = make_resolve_sym(93, "User", "Models.User", "struct", "Sources/App/Models/User.swift");
    let b = make_resolve_sym(94, "User", "Dto.User", "struct", "Sources/App/Dto/User.swift");
    let fix = ByNameFixture::with("User", vec![a, b]);
    let file_ctx = swift_file_ctx("Sources/App/Handlers/Use.swift");
    let source_sym = make_resolve_source("use", "use");
    let extracted = make_typeref("User");
    let ref_ctx = module_scope_ctx(&extracted, &source_sym);

    let res = (DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: &fix,
        kind_compatible: super::predicates::kind_compatible,
    })
    .resolve_all_with_profile(&super::profile::SWIFT_PROFILE);
    let strategy = res.map(|r| r.strategy).unwrap_or("");
    assert_ne!(
        strategy, "default_module_scope",
        "two in-module candidates are ambiguous — decline"
    );
}

/// Soundness: files with no `Sources/<Target>/` prefix (a flat layout) leave
/// the `SourcesTargetSubtree` boundary undefined → the rung is inert. It is
/// NOT a same-dir fallback: a same-parent-dir candidate off-layout still does
/// not bind through the module-scope rung.
#[test]
fn swift_same_module_declines_outside_sources_layout() {
    let user = make_resolve_sym(95, "User", "User", "struct", "App/User.swift");
    let fix = ByNameFixture::with("User", vec![user]);
    let file_ctx = swift_file_ctx("App/Use.swift");
    let source_sym = make_resolve_source("use", "use");
    let extracted = make_typeref("User");
    let ref_ctx = module_scope_ctx(&extracted, &source_sym);

    let res = (DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: &fix,
        kind_compatible: super::predicates::kind_compatible,
    })
    .resolve_all_with_profile(&super::profile::SWIFT_PROFILE);
    let strategy = res.map(|r| r.strategy).unwrap_or("");
    assert_ne!(
        strategy, "default_module_scope",
        "SourcesTargetSubtree is inert off the Sources/<Target>/ layout"
    );
}

impl SymbolLookup for ByNameFixture {
    fn by_name(&self, name: &str) -> &[SymbolInfo] {
        self.by_name_map.get(name).map(|v| v.as_slice()).unwrap_or(&self.empty)
    }
    fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> { None }
    fn members_of(&self, _: &str) -> &[SymbolInfo] { &self.empty }
    fn types_by_name(&self, _: &str) -> &[SymbolInfo] { &self.empty }
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> { Vec::new() }
    fn has_in_namespace(&self, _: &str) -> bool { false }
    fn in_file(&self, _: &str) -> &[SymbolInfo] { &self.empty }
    fn field_type_name(&self, _: &str) -> Option<&str> { None }
    fn return_type_name(&self, _: &str) -> Option<&str> { None }
    fn field_type_args(&self, _: &str) -> Option<&[String]> { None }
    fn generic_params(&self, _: &str) -> Option<&[String]> { None }
    fn reexports_from(&self, _: &str) -> &[(String, String)] { &self.empty_reexports }
    fn is_external_name(&self, _: &str, _: &str) -> bool { false }
}

fn make_resolve_sym(id: i64, name: &str, qname: &str, kind: &str, path: &str) -> SymbolInfo {
    SymbolInfo {
        id,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: kind.to_string(),
        visibility: Some("public".to_string()),
        file_path: Arc::from(path),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

fn make_resolve_source(name: &str, qname: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: SymbolKind::Function,
        visibility: None,
        start_line: 0,
        end_line: 0,
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

fn make_typeref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::TypeRef,
        line: 1,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
        chain: None,
        byte_offset: 1,
    }
}

/// `import struct MyModule.Bar` then a bare `Bar` type ref. The import's
/// `module_path` is dotted (`MyModule.Bar`) and its `imported_name` is the
/// path's last segment `Bar` == the ref target. The unique internal struct
/// `Bar` binds via the new scope-directed strategy.
///
/// Pre-cut this returns None: `resolve_via_file_import`'s
/// `file_path_matches_module` gate rejects `MyModule.Bar` against
/// `Sources/MyModule/Bar.swift`, and no other ladder rung matches a bare
/// `Bar` with no chain / no module on the ref.
#[test]
fn swift_explicit_member_import_binds_unique_internal_struct() {
    let bar = make_resolve_sym(42, "Bar", "Bar", "struct", "Sources/MyModule/Bar.swift");
    let fix = ByNameFixture::with("Bar", vec![bar]);

    let file_ctx = FileContext {
        file_path: "Sources/App/Use.swift".to_string(),
        language: "swift".to_string(),
        imports: vec![ImportEntry {
            imported_name: "Bar".to_string(),
            module_path: Some("MyModule.Bar".to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    };
    let source_sym = make_resolve_source("use", "use");
    let extracted = make_typeref("Bar");
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: Vec::new(),
        file_package_id: None,
    };

    let res = (DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: &fix,
        kind_compatible: super::predicates::kind_compatible,
    })
    .resolve_all_with_profile(&super::profile::SWIFT_PROFILE)
    .expect("explicit-member import binds the unique internal struct");
    assert_eq!(res.strategy, "default_explicit_member_import");
    assert_eq!(res.target_symbol_id, 42);
}

/// Soundness boundary: a NON-dotted `module_path` (plain `import Foundation`)
/// plus a coincidental internal `Foundation`-named symbol must NOT bind via
/// this strategy. A plain whole-module import carries no project-symbol scope
/// evidence, so it stays out of the cut (left for external classification).
#[test]
fn swift_plain_module_import_does_not_arm_explicit_member_strategy() {
    let coincidental =
        make_resolve_sym(7, "Foundation", "Foundation", "struct", "Sources/App/Foundation.swift");
    let fix = ByNameFixture::with("Foundation", vec![coincidental]);

    let file_ctx = FileContext {
        file_path: "Sources/App/Use.swift".to_string(),
        language: "swift".to_string(),
        imports: vec![ImportEntry {
            imported_name: "Foundation".to_string(),
            module_path: Some("Foundation".to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    };
    let source_sym = make_resolve_source("use", "use");
    let extracted = make_typeref("Foundation");
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: Vec::new(),
        file_package_id: None,
    };

    let res = (DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: &fix,
        kind_compatible: super::predicates::kind_compatible,
    })
    .resolve_all_with_profile(&super::profile::SWIFT_PROFILE);
    let strategy = res.map(|r| r.strategy).unwrap_or("");
    assert_ne!(
        strategy, "default_explicit_member_import",
        "plain whole-module import must not arm the explicit-member strategy"
    );
}

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
