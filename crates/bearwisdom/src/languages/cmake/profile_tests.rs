// =============================================================================
// cmake/profile_tests.rs — profile-axis, kind-table, and full-ladder bind tests.
//
// CMake variables and functions/macros are project-global: they flow down
// `add_subdirectory`, so a bare `${VAR}` or command ref binds across the build
// tree. `namespaceless_global_type_lookup == Global` drives that bind via the
// dead-last first-match-by-name rung; a same-named ext: toolchain stub declines
// and stays external.
// =============================================================================

use super::CMAKE_PROFILE;
use crate::indexer::resolve::engine::contract::{FileContext, RefContext, Resolution};
use crate::indexer::resolve::engine::compilation::Compilation;
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::language_profile::{KindCompatibility, NamespaceScope};
use crate::types::*;
use std::collections::HashMap;

#[test]
fn cmake_profile_identity_and_shadow_mode() {
    assert_eq!(CMAKE_PROFILE.id, "cmake");
}

#[test]
fn cmake_namespaceless_global_is_on() {
    // CMake variables/functions are build-tree global, so a bare ref binds via
    // the dead-last first-match-by-name rung.
    assert_eq!(
        CMAKE_PROFILE.namespaceless_global_type_lookup,
        NamespaceScope::Global
    );
}

#[test]
fn cmake_kind_table_matches_former_predicate() {
    let t = CMAKE_PROFILE.kind_compatible_table;
    // Calls → function (macros extract as Function).
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Function
    ));
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Variable
    ));
    // TypeRef → variable | function.
    assert!(KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Variable
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Function
    ));
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Class
    ));
    // Unlisted edge kinds stay permissive.
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Imports,
        SymbolKind::Namespace
    ));
}

// ---------------------------------------------------------------------------
// Full-ladder bind tests through resolve_all_with_profile(&CMAKE_PROFILE).
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
        language: "cmake".to_string(),
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

fn build_index(files: &[&ParsedFile]) -> (Compilation, HashMap<(String, String), i64>) {
    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in files {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }
    let owned: Vec<ParsedFile> = files.iter().map(|f| clone_pf(f)).collect();
    let index = Compilation::build(&owned, &id_map, std::sync::Arc::new(crate::type_checker::core::types::TypeArena::new()));
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
        vec![make_sym("caller", SymbolKind::Function)],
        vec![make_ref(target, kind)],
    );
    let file_ctx = FileContext {
        file_path: file_path.to_string(),
        language: "cmake".to_string(),
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
    .resolve_all_with_profile(&CMAKE_PROFILE)
}

#[test]
fn cmake_bare_variable_binds_internal_over_external() {
    // `MY_FLAGS` is set in two project `.cmake` files (it flows down the build
    // tree) plus a same-named ext: toolchain stub. A bare `${MY_FLAGS}` ref
    // binds an INTERNAL definition first-match; the external stub loses.
    let a = make_file(
        "cmake/flags.cmake",
        vec![make_sym("MY_FLAGS", SymbolKind::Variable)],
        vec![],
    );
    let b = make_file(
        "src/CMakeLists.txt",
        vec![make_sym("MY_FLAGS", SymbolKind::Variable)],
        vec![],
    );
    let ext = make_file(
        "ext:cmake:toolchain/flags.cmake",
        vec![make_sym("MY_FLAGS", SymbolKind::Variable)],
        vec![],
    );
    let (id_a, id_b, ext_id) = {
        let (_, id_map) = build_index(&[&a, &b, &ext]);
        (
            sym_id(&id_map, "cmake/flags.cmake", "MY_FLAGS"),
            sym_id(&id_map, "src/CMakeLists.txt", "MY_FLAGS"),
            sym_id(&id_map, "ext:cmake:toolchain/flags.cmake", "MY_FLAGS"),
        )
    };
    let res = resolve_ref("CMakeLists.txt", "MY_FLAGS", EdgeKind::TypeRef, &[&a, &b, &ext])
        .expect("bare CMake variable ref binds an internal definition");
    assert_eq!(res.strategy, "default_namespaceless_global");
    assert_ne!(
        res.target_symbol_id, ext_id,
        "must not bind the ext toolchain stub"
    );
    assert!(
        res.target_symbol_id == id_a || res.target_symbol_id == id_b,
        "binds an internal MY_FLAGS (got {})",
        res.target_symbol_id
    );
}

#[test]
fn cmake_external_only_name_stays_unresolved() {
    // `CMAKE_CXX_STANDARD` is owned ONLY by an external toolchain file — no
    // project definition. The internal-only rung declines, leaving it for
    // external classification.
    let ext = make_file(
        "ext:cmake:toolchain/std.cmake",
        vec![make_sym("CMAKE_CXX_STANDARD", SymbolKind::Variable)],
        vec![],
    );
    let res = resolve_ref("CMakeLists.txt", "CMAKE_CXX_STANDARD", EdgeKind::TypeRef, &[&ext]);
    assert!(
        res.is_none(),
        "external-only name must not bind an internal symbol; got: {res:?}"
    );
}
