use std::collections::HashMap;

use super::predicates;
use super::C_LANG_PROFILE;
use crate::indexer::resolve::engine::{
    build_scope_chain, FileContext, RefContext, Resolution, SymbolIndex,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility,
};

#[test]
fn c_profile_identity_and_shadow_mode() {
    assert_eq!(C_LANG_PROFILE.id, "c");
}

/// `Calls → Variable` is admitted: C invokes callables through Variable symbols
/// (function pointers and object-like `#define` aliases of a callable). Function
/// and Method stay admitted; an Enum target stays refused, so the widening is
/// confined to the kind C legitimately calls through.
#[test]
fn c_calls_admits_variable_for_fnptr_and_macro_alias() {
    let t = C_LANG_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Variable
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Function
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Method
    ));
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Enum
    ));
}

/// A `TypeRef` to a Variable stays refused even though `Calls → Variable` is now
/// admitted — the Variable widening is scoped to the Calls row alone.
#[test]
fn c_typeref_still_refuses_variable() {
    let t = C_LANG_PROFILE.kind_compatible_table;
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Variable
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Struct
    ));
}

fn make_symbol(name: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind,
        visibility: Some(Visibility::Public),
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

fn calls_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_file(path: &str, syms: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "c".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: syms,
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

fn build_env(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
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

/// Drive the chain-less bare-name ladder the way production does for a single
/// call ref — gated by the profile's `kind_compatible_table`.
fn resolve(source: &ParsedFile, index: &SymbolIndex) -> Option<Resolution> {
    let file_ctx = FileContext {
        file_path: source.path.clone(),
        language: "c".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let ref_ctx = RefContext {
        extracted_ref: &source.refs[0],
        source_symbol: &source.symbols[0],
        scope_chain: build_scope_chain(None),
        file_package_id: None,
    };
    DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: index,
        kind_compatible: |_, _| true,
    }
    .resolve_all_with_profile(&C_LANG_PROFILE)
}

/// `#define ngx_free free` is emitted as a Variable symbol; the bare call
/// `ngx_free(p)` binds to it now that the Calls row admits Variable.
#[test]
fn macro_alias_call_binds_to_variable_symbol() {
    let file = make_file(
        "core.c",
        vec![
            make_symbol("f", SymbolKind::Function),
            make_symbol("ngx_free", SymbolKind::Variable),
        ],
        vec![calls_ref("ngx_free")],
    );
    let (index, id_map) = build_env(&[&file]);
    let res = resolve(&file, &index).expect("macro-alias call should bind to the Variable symbol");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("core.c".to_string(), "ngx_free".to_string()))
            .unwrap()
    );
}

/// A function pointer `void (*fp)(void)` is extracted as a Variable; the call
/// `fp()` binds through that binding.
#[test]
fn function_pointer_call_binds_to_variable_symbol() {
    let file = make_file(
        "main.c",
        vec![
            make_symbol("main", SymbolKind::Function),
            make_symbol("fp", SymbolKind::Variable),
        ],
        vec![calls_ref("fp")],
    );
    let (index, id_map) = build_env(&[&file]);
    let res = resolve(&file, &index).expect("function-pointer call should bind to the Variable");
    assert_eq!(
        res.target_symbol_id,
        *id_map
            .get(&("main.c".to_string(), "fp".to_string()))
            .unwrap()
    );
}

/// A `Calls` ref must not bind to a same-named Enum — the widening admits
/// Variable only, not every non-callable kind.
#[test]
fn call_does_not_bind_to_unrelated_enum_kind() {
    let file = make_file(
        "color.c",
        vec![
            make_symbol("main", SymbolKind::Function),
            make_symbol("Color", SymbolKind::Enum),
        ],
        vec![calls_ref("Color")],
    );
    let (index, _id_map) = build_env(&[&file]);
    assert!(
        resolve(&file, &index).is_none(),
        "a Calls ref must not bind to an Enum target"
    );
}

/// The plugin's `keywords()` exposes the full C/C++/POSIX spec set (the data
/// the resolver's external classifier consumes), not the 12-entry primitive
/// stub. Stdlib callees decline as builtins through this set.
#[test]
fn c_keywords_redirect_exposes_stdlib_set() {
    let kw = crate::indexer::keywords::keywords_for_language("c");
    assert!(
        kw.contains(&"strlen"),
        "keywords() must expose the full spec set (strlen)"
    );
    assert!(kw.contains(&"malloc"));
    assert!(kw.contains(&"fprintf"));
    // The purged nlohmann/json template-param name must not survive as a keyword.
    assert!(
        !kw.contains(&"BasicJsonType"),
        "project-specific template-param names must be purged from keywords()"
    );
    // `cpp` routes to the same plugin and therefore the same set.
    assert!(crate::indexer::keywords::keywords_for_language("cpp").contains(&"strlen"));
}

/// Purged template-param names stay suppressed through the generic
/// `is_template_param` predicate, which covers the patterns they matched
/// (the `EndsWith("Type")` rule and the `<Word>T` convention).
#[test]
fn purged_template_params_covered_by_predicate() {
    // `EndsWith("Type")` rule.
    assert!(predicates::is_template_param("BasicJsonType"));
    assert!(predicates::is_template_param("IteratorType"));
    assert!(predicates::is_template_param("AllocatorType"));
    assert!(predicates::is_template_param("ValueType"));
    assert!(predicates::is_template_param("ConstructibleArrayType"));
    // `<Word>T` convention.
    assert!(predicates::is_template_param("LhsT"));
    assert!(predicates::is_template_param("RhsT"));
    assert!(predicates::is_template_param("ArgT"));
    // The convention must not swallow ordinary type names that merely end in `T`
    // (all-caps acronyms / shouty constants are not `<Word>T`).
    assert!(!predicates::is_template_param("SAX"));
    assert!(!predicates::is_template_param("UINT"));
}

#[test]
fn c_profile_namespace_decline_gates_r_c_api() {
    // The R-package C-API decline is namespace-gated profile data: armed by the
    // R-package file namespace, reserves the R C API symbol set.
    let nd = C_LANG_PROFILE
        .namespace_decline
        .expect("c profile declares a namespace decline");
    assert_eq!(
        nd.file_namespace,
        crate::languages::c_lang::hooks::R_PACKAGE_SENTINEL
    );
    assert!((nd.is_reserved)("Rf_eval"));
    assert!(!(nd.is_reserved)("my_project_fn"));
}
