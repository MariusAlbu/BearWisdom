// =============================================================================
// proto/profile_tests.rs — profile-axis, kind-table, and full-ladder binds.
//
// Protobuf messages/enums share one flat package namespace across `.proto`
// files, so a bare cross-file message/enum type ref binds to its declaration.
// `namespaceless_global_type_lookup == Global` drives that bind via the
// dead-last first-match-by-name rung; a same-named ext: well-known-type stub
// declines and stays external.
// =============================================================================

use super::PROTO_PROFILE;
use crate::indexer::resolve::legacy::{FileContext, RefContext, Resolution, SymbolIndex};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::language_profile::{KindCompatibility, NamespaceScope};
use crate::types::*;
use std::collections::HashMap;

#[test]
fn proto_profile_identity_and_shadow_mode() {
    assert_eq!(PROTO_PROFILE.id, "proto");
}

#[test]
fn proto_namespaceless_global_is_on() {
    // Messages/enums are package-flat across `.proto` files, so a bare type ref
    // binds via the dead-last first-match-by-name rung.
    assert_eq!(
        PROTO_PROFILE.namespaceless_global_type_lookup,
        NamespaceScope::Global
    );
}

#[test]
fn proto_type_ref_accepts_message_enum_kinds() {
    let t = PROTO_PROFILE.kind_compatible_table;
    for k in [SymbolKind::Struct, SymbolKind::Enum, SymbolKind::Class] {
        assert!(KindCompatibility::check(t, EdgeKind::TypeRef, k));
    }
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Function
    ));
}

// ---------------------------------------------------------------------------
// Full-ladder bind tests through resolve_all_with_profile(&PROTO_PROFILE).
// ---------------------------------------------------------------------------

fn accept_any(_edge: EdgeKind, _sym_kind: &str) -> bool {
    true
}

fn make_sym(name: &str, kind: SymbolKind) -> ExtractedSymbol {
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

fn make_ref(target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind,
        line: 2,
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
        language: "proto".to_string(),
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

fn clone_pf(f: &ParsedFile) -> ParsedFile {
    make_file(&f.path, f.symbols.clone(), f.refs.clone())
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
    let owned: Vec<ParsedFile> = files.iter().map(|f| clone_pf(f)).collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

fn sym_id(id_map: &HashMap<(String, String), i64>, file: &str, name: &str) -> i64 {
    *id_map
        .get(&(file.to_string(), name.to_string()))
        .unwrap_or_else(|| panic!("symbol not found: {file}::{name}"))
}

fn resolve_ref(file_path: &str, target: &str, kind: EdgeKind, all: &[&ParsedFile]) -> Option<Resolution> {
    let (index, _) = build_index(all);
    let caller = make_file(
        file_path,
        vec![make_sym("Request", SymbolKind::Struct)],
        vec![make_ref(target, kind)],
    );
    let file_ctx = FileContext {
        file_path: file_path.to_string(),
        language: "proto".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let r = &caller.refs[0];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &caller.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };
    DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: &index,
        kind_compatible: accept_any,
    }
    .resolve_all_with_profile(&PROTO_PROFILE)
}

#[test]
fn proto_bare_message_binds_internal_over_external() {
    // `User` is the project's own message (declared in two `.proto` files) plus
    // a same-named ext: well-known-type stub. A bare cross-file type ref binds
    // an INTERNAL declaration first-match; the external stub loses.
    let a = make_file(
        "proto/user.proto",
        vec![make_sym("User", SymbolKind::Struct)],
        vec![],
    );
    let b = make_file(
        "proto/account.proto",
        vec![make_sym("User", SymbolKind::Struct)],
        vec![],
    );
    let ext = make_file(
        "ext:proto:google/user.proto",
        vec![make_sym("User", SymbolKind::Struct)],
        vec![],
    );
    let (id_a, id_b, ext_id) = {
        let (_, id_map) = build_index(&[&a, &b, &ext]);
        (
            sym_id(&id_map, "proto/user.proto", "User"),
            sym_id(&id_map, "proto/account.proto", "User"),
            sym_id(&id_map, "ext:proto:google/user.proto", "User"),
        )
    };
    let res = resolve_ref("proto/service.proto", "User", EdgeKind::TypeRef, &[&a, &b, &ext])
        .expect("bare proto message ref binds an internal declaration");
    assert_eq!(res.strategy, "default_namespaceless_global");
    assert_ne!(
        res.target_symbol_id, ext_id,
        "must not bind the ext well-known-type stub"
    );
    assert!(
        res.target_symbol_id == id_a || res.target_symbol_id == id_b,
        "binds an internal User (got {})",
        res.target_symbol_id
    );
}

#[test]
fn proto_external_only_message_stays_unresolved() {
    // `Timestamp` is owned ONLY by an external well-known-type file — no
    // project declaration. The internal-only rung declines, leaving it for
    // external classification.
    let ext = make_file(
        "ext:proto:google/protobuf/timestamp.proto",
        vec![make_sym("Timestamp", SymbolKind::Struct)],
        vec![],
    );
    let res = resolve_ref("proto/service.proto", "Timestamp", EdgeKind::TypeRef, &[&ext]);
    assert!(
        res.is_none(),
        "external-only message must not bind an internal symbol; got: {res:?}"
    );
}
