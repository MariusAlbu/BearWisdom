// =============================================================================
// type_checker/engine_tests.rs — Engine façade end-to-end tests.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolInfo, SymbolLookup};
use crate::languages::typescript::extract;
use crate::type_checker::core::{SymbolIdMap, TypeArena};
use crate::languages::typescript::TYPESCRIPT_PROFILE;
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::{
    AliasTarget, ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, ParsedFile,
    SegmentKind, SymbolKind, Visibility,
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

fn ts_parsed_file(path: &str, source: &str) -> ParsedFile {
    let extraction = extract::extract(source, false);
    ParsedFile {
        path: path.to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: source.len() as u64,
        line_count: source.lines().count() as u32,
        mtime: None,
        package_id: None,
        symbols: extraction.symbols,
        refs: extraction.refs,
        routes: extraction.routes,
        db_sets: extraction.db_sets,
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: Some(source.to_string()),
        has_errors: extraction.has_errors,
        flow: Default::default(),
        demand_contributions: extraction.demand_contributions,
        alias_targets: extraction.alias_targets,
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn deterministic_ids(pf: &ParsedFile) -> SymbolIdMap {
    let mut map = SymbolIdMap::default();
    for (idx, _) in pf.symbols.iter().enumerate() {
        map.insert((pf.path.clone(), idx), idx as i64 + 1);
    }
    map
}

struct EmptyLookup {
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
    types: rustc_hash::FxHashMap<String, Vec<SymbolInfo>>,
}

impl EmptyLookup {
    fn from(pf: &ParsedFile, sym_ids: &SymbolIdMap) -> Self {
        let mut types: rustc_hash::FxHashMap<String, Vec<SymbolInfo>> = Default::default();
        let file_path: Arc<str> = Arc::from(pf.path.as_str());
        for (idx, sym) in pf.symbols.iter().enumerate() {
            if !matches!(
                sym.kind,
                SymbolKind::Class
                    | SymbolKind::Interface
                    | SymbolKind::Struct
                    | SymbolKind::Trait
                    | SymbolKind::Enum
                    | SymbolKind::TypeAlias
            ) {
                continue;
            }
            let id = sym_ids
                .get(&(pf.path.clone(), idx))
                .copied()
                .unwrap_or(idx as i64 + 1);
            types.entry(sym.name.clone()).or_default().push(SymbolInfo {
                id,
                name: sym.name.clone(),
                qualified_name: sym.qualified_name.clone(),
                kind: sym.kind.as_str().to_string(),
                visibility: sym.visibility.map(|v| v.as_str().to_string()),
                file_path: file_path.clone(),
                scope_path: sym.scope_path.clone(),
                package_id: pf.package_id,
                signature: sym.signature.clone(),
            });
        }
        Self {
            empty: Vec::new(),
            empty_reexports: Vec::new(),
            types,
        }
    }
}

impl SymbolLookup for EmptyLookup {
    fn by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> {
        None
    }
    fn members_of(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn types_by_name(&self, name: &str) -> &[SymbolInfo] {
        self.types.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn field_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn field_type_args(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn generic_params(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn alias_target(&self, _: &str) -> Option<&AliasTarget> {
        None
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_reexports
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

fn ref_ctx_for<'a>(
    extracted_ref: &'a ExtractedRef,
    source_symbol: &'a ExtractedSymbol,
) -> RefContext<'a> {
    RefContext {
        extracted_ref,
        source_symbol,
        scope_chain: Vec::new(),
        file_package_id: None,
    }
}

fn dummy_source() -> ExtractedSymbol {
    ExtractedSymbol {
        name: "caller".to_string(),
        qualified_name: "caller".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn file_ctx_ts(path: &str) -> FileContext {
    FileContext {
        file_path: path.to_string(),
        language: "typescript".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    }
}

#[test]
fn engine_is_send_and_sync() {
    // The resolver loop runs in parallel via rayon; the engine state must
    // be safely shareable across workers. Compile-time assertion.
    fn require_send_sync<T: Send + Sync>() {}
    require_send_sync::<Engine<'static>>();
}

#[test]
fn engine_resolve_returns_none_when_ref_has_no_chain() {
    let pf = ts_parsed_file(
        "src/u.ts",
        "export class User { name: string = \"\"; }",
    );
    let sym_ids = deterministic_ids(&pf);
    let lookup = EmptyLookup::from(&pf, &sym_ids);

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", &DEFAULT_PROFILE);

    let mut engine = Engine::build(std::slice::from_ref(&pf), &sym_ids, profiles, &lookup);

    let source = dummy_source();
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: 0,
        target_name: "x".to_string(),
        kind: EdgeKind::Reads,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None, // no chain → engine declines, legacy path takes over
        byte_offset: 0,
        call_args: Vec::new(),
    };
    let rc = ref_ctx_for(&r, &source);
    let fc = file_ctx_ts("src/u.ts");
    assert!(engine.resolve(&rc, &fc, &lookup).is_none());
}

#[test]
fn engine_resolve_returns_none_for_unregistered_language() {
    let pf = ts_parsed_file("src/u.ts", "export class User {}");
    let sym_ids = deterministic_ids(&pf);
    let lookup = EmptyLookup::from(&pf, &sym_ids);

    // No profiles registered → engine.resolve declines every ref.
    let profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    let mut engine = Engine::build(std::slice::from_ref(&pf), &sym_ids, profiles, &lookup);

    let chain = MemberChain {
        segments: vec![ChainSegment {
            name: "User".to_string(),
            node_kind: "type_identifier".to_string(),
            kind: SegmentKind::TypeAccess,
            declared_type: None,
            type_args: Vec::new(),
            optional_chaining: false,
            byte_offset: 0,
            declared_type_id: None,
            is_call: false,
            type_arg_ids: Vec::new(),
        }],
    };
    let source = dummy_source();
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: 0,
        target_name: "User".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: Some(chain),
        byte_offset: 0,
        call_args: Vec::new(),
    };
    let rc = ref_ctx_for(&r, &source);
    let fc = file_ctx_ts("src/u.ts");
    assert!(engine.resolve(&rc, &fc, &lookup).is_none());
}

#[test]
fn engine_build_from_registry_collects_typescript_hooks() {
    // Smoke test: build_from_registry walks default_registry().all() and
    // collects every plugin's `language_hooks()`. TypeScriptPlugin returns
    // Some(&TYPESCRIPT_HOOKS); a registered "typescript" language id must
    // therefore resolve through hooks_for. Reachable via tsx too since the
    // TS plugin claims both language ids.
    let pf = ts_parsed_file("src/u.ts", "export class User {}");
    let sym_ids = deterministic_ids(&pf);
    let lookup = EmptyLookup::from(&pf, &sym_ids);

    let engine = Engine::build_from_registry(
        std::slice::from_ref(&pf),
        &sym_ids,
        &lookup,
        std::sync::Arc::new(crate::type_checker::core::types::TypeArena::new()),
    );
    assert!(
        engine.hooks_for("typescript").is_some(),
        "TypeScriptPlugin::language_hooks() should be collected into the engine"
    );
    assert!(engine.hooks_for("tsx").is_some());
    assert!(engine.hooks_for("nonexistent_language").is_none());
}

#[test]
fn engine_resolve_walks_single_segment_chain_to_self_yielding_class() {
    // The TS extractor emits a Class for `export class User {}`; engine.build
    // populates SymbolTypeMap's self-yield reverse index; engine.resolve on a
    // bare TypeAccess chain to User returns the class's sym id. Uses the
    // real TYPESCRIPT_PROFILE so engine.resolve runs the chain walker.
    let pf = ts_parsed_file("src/u.ts", "export class User {}");
    let sym_ids = deterministic_ids(&pf);
    let lookup = EmptyLookup::from(&pf, &sym_ids);

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", &TYPESCRIPT_PROFILE);

    let mut engine = Engine::build(std::slice::from_ref(&pf), &sym_ids, profiles, &lookup);

    let user_idx = pf
        .symbols
        .iter()
        .position(|s| s.name == "User" && s.kind == SymbolKind::Class)
        .expect("User class extracted");
    let user_id = sym_ids[&(pf.path.clone(), user_idx)];

    let chain = MemberChain {
        segments: vec![ChainSegment {
            name: "User".to_string(),
            node_kind: "type_identifier".to_string(),
            kind: SegmentKind::TypeAccess,
            declared_type: None,
            type_args: Vec::new(),
            optional_chaining: false,
            byte_offset: 0,
            declared_type_id: None,
            is_call: false,
            type_arg_ids: Vec::new(),
        }],
    };
    let source = dummy_source();
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: 0,
        target_name: "User".to_string(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: Some(chain),
        byte_offset: 0,
        call_args: Vec::new(),
    };
    let rc = ref_ctx_for(&r, &source);
    let fc = file_ctx_ts("src/u.ts");

    let resolution = engine.resolve(&rc, &fc, &lookup).expect("engine resolves User");
    assert_eq!(resolution.target_symbol_id, user_id);
    let user_ty = engine.arena().class("User");
    assert_eq!(resolution.resolved_yield_type, Some(user_ty));
    assert_eq!(resolution.strategy, "engine_chain_root");
}

#[test]
fn engine_yields_none_when_last_segment_has_no_type() {
    // `class User { bar() {} }` — bar has no captured return type, so the
    // chain resolves bar but the last segment's yield is Unknown. The adapter
    // must surface None, not a yield that formats to "unknown" — otherwise a
    // forward-inferred local (`let w = u.bar(); w.x`) is typed `unknown` and
    // every later member access on it fails.
    let pf = ts_parsed_file("src/u.ts", "export class User { bar() {} }");
    let sym_ids = deterministic_ids(&pf);
    let lookup = EmptyLookup::from(&pf, &sym_ids);

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", &TYPESCRIPT_PROFILE);
    let mut engine = Engine::build(std::slice::from_ref(&pf), &sym_ids, profiles, &lookup);

    let bar_idx = pf
        .symbols
        .iter()
        .position(|s| s.name == "bar")
        .expect("bar method extracted");
    let bar_id = sym_ids[&(pf.path.clone(), bar_idx)];

    let chain = MemberChain {
        segments: vec![
            ChainSegment {
                name: "User".to_string(),
                node_kind: "type_identifier".to_string(),
                kind: SegmentKind::TypeAccess,
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                type_arg_ids: Vec::new(),
            },
            ChainSegment {
                name: "bar".to_string(),
                node_kind: "property_identifier".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                type_arg_ids: Vec::new(),
            },
        ],
    };
    let source = dummy_source();
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: 0,
        target_name: "bar".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: Some(chain),
        byte_offset: 0,
        call_args: Vec::new(),
    };
    let rc = ref_ctx_for(&r, &source);
    let fc = file_ctx_ts("src/u.ts");

    let resolution = engine.resolve(&rc, &fc, &lookup).expect("engine resolves bar");
    assert_eq!(resolution.target_symbol_id, bar_id);
    assert_eq!(resolution.resolved_yield_type, None);
}

#[test]
fn engine_build_aggregates_aliases_into_index() {
    let pf = ts_parsed_file(
        "src/a.ts",
        "export type Id = string;\nexport type Hand = Id;\n",
    );
    let sym_ids = deterministic_ids(&pf);
    let lookup = EmptyLookup::from(&pf, &sym_ids);

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", &DEFAULT_PROFILE);

    let engine = Engine::build(std::slice::from_ref(&pf), &sym_ids, profiles, &lookup);
    assert!(
        !engine.aliases().is_empty(),
        "TS extractor emits alias_targets for `type X = Y` decls"
    );
}

// =============================================================================
// BIND-4 — bare-name overload disambiguation by argument arity / type.
// =============================================================================

/// Lookup double for the bare-overload tests. Holds two same-name callables
/// reachable via both `by_name` (the overload-set source) and `in_file` (the
/// bare resolver's first-match same-file path), so the engine resolves the ref
/// to the first-indexed `foo` and BIND-4 must re-select by arity/type.
struct OverloadLookup {
    foos: Vec<SymbolInfo>,
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl OverloadLookup {
    fn new(foos: Vec<SymbolInfo>) -> Self {
        Self {
            foos,
            empty: Vec::new(),
            empty_reexports: Vec::new(),
        }
    }
}

impl SymbolLookup for OverloadLookup {
    fn by_name(&self, name: &str) -> &[SymbolInfo] {
        if name == "foo" {
            &self.foos
        } else {
            &self.empty
        }
    }
    fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> {
        None
    }
    fn members_of(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn types_by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> &[SymbolInfo] {
        &self.foos
    }
    fn field_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn field_type_args(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn generic_params(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn alias_target(&self, _: &str) -> Option<&AliasTarget> {
        None
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_reexports
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

/// Build the two `foo` callables, each interned into `arena` with the given
/// per-symbol parameter types, plus the matching ParsedFile + SymbolTypeMap
/// keys. Ids are 1 and 2 in declaration order so the bare resolver's
/// first-match lands on id 1.
fn foo_overloads(
    arena: &TypeArena,
    params_a: Vec<crate::type_checker::core::types::TypeId>,
    params_b: Vec<crate::type_checker::core::types::TypeId>,
) -> (ParsedFile, SymbolIdMap, Vec<SymbolInfo>) {
    let _ = arena;
    let mk_sym = |params: Vec<crate::type_checker::core::types::TypeId>| ExtractedSymbol {
        name: "foo".to_string(),
        qualified_name: "foo".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: params,
        generic_params: Vec::new(),
    };
    let symbols = vec![mk_sym(params_a), mk_sym(params_b)];
    let pf = ParsedFile {
        path: "src/f.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: symbols.clone(),
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: Default::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };
    let mut sym_ids = SymbolIdMap::default();
    sym_ids.insert(("src/f.ts".to_string(), 0), 1);
    sym_ids.insert(("src/f.ts".to_string(), 1), 2);

    let file_path: Arc<str> = Arc::from(pf.path.as_str());
    let infos: Vec<SymbolInfo> = symbols
        .iter()
        .enumerate()
        .map(|(idx, s)| SymbolInfo {
            id: idx as i64 + 1,
            name: s.name.clone(),
            qualified_name: s.qualified_name.clone(),
            kind: s.kind.as_str().to_string(),
            visibility: s.visibility.map(|v| v.as_str().to_string()),
            file_path: file_path.clone(),
            scope_path: s.scope_path.clone(),
            package_id: None,
            signature: None,
        })
        .collect();
    (pf, sym_ids, infos)
}

fn bare_call_ref(arg_count: usize) -> ExtractedRef {
    let call_args = (0..arg_count)
        .map(|i| crate::types::CallArg::Literal((i + 1).to_string()))
        .collect();
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: "foo".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args,
    }
}

#[test]
fn bare_name_call_overload_selected_by_arity() {
    // Two `foo`: foo(a) and foo(a, b). A 2-arg call must retarget from the
    // first-indexed foo (id 1, arity 1) to the 2-arg foo (id 2) under the
    // `bare_overload_arg_typed` strategy — arity uniquely selects it.
    let arena = Arc::new(TypeArena::new());
    let int_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Int);
    let (pf, sym_ids, infos) = foo_overloads(&arena, vec![int_ty], vec![int_ty, int_ty]);
    let lookup = OverloadLookup::new(infos);

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", &TYPESCRIPT_PROFILE);
    let engine = Engine::build_with_hooks(
        std::slice::from_ref(&pf),
        &sym_ids,
        profiles,
        FxHashMap::default(),
        &lookup,
        arena.clone(),
    );

    let source = dummy_source();
    let r = bare_call_ref(2);
    let rc = ref_ctx_for(&r, &source);
    let fc = file_ctx_ts("src/f.ts");

    let resolution = engine.resolve(&rc, &fc, &lookup).expect("bare foo resolves");
    assert_eq!(
        resolution.target_symbol_id, 2,
        "2-arg call must select the 2-arg foo, not the first-indexed foo"
    );
    assert_eq!(resolution.strategy, "bare_overload_arg_typed");
}

#[test]
fn bare_name_call_same_arity_both_assignable_keeps_first_match() {
    // Two `foo(a)`, both with an Int param. A single Int arg is assignable to
    // both → ambiguous → no override, keep the first-match (id 1, the bare
    // resolver's same-file strategy).
    let arena = Arc::new(TypeArena::new());
    let int_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Int);
    let (pf, sym_ids, infos) = foo_overloads(&arena, vec![int_ty], vec![int_ty]);
    let lookup = OverloadLookup::new(infos);

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", &TYPESCRIPT_PROFILE);
    let engine = Engine::build_with_hooks(
        std::slice::from_ref(&pf),
        &sym_ids,
        profiles,
        FxHashMap::default(),
        &lookup,
        arena.clone(),
    );

    let source = dummy_source();
    let r = bare_call_ref(1);
    let rc = ref_ctx_for(&r, &source);
    let fc = file_ctx_ts("src/f.ts");

    let resolution = engine.resolve(&rc, &fc, &lookup).expect("bare foo resolves");
    assert_eq!(
        resolution.target_symbol_id, 1,
        "two same-arity assignable overloads stay ambiguous → first-match kept"
    );
    assert_ne!(resolution.strategy, "bare_overload_arg_typed");
}

#[test]
fn bare_name_call_overload_ignores_out_of_scope_homonym() {
    // foo(a) [id 1] in src/f.ts is the in-scope first-match; foo(a, b) [id 2]
    // lives in a DIFFERENT file. A 2-arg call's arity uniquely matches id 2, but
    // id 2 is out of the first-match's scope — the scope filter must reject it and
    // keep id 1. Without the filter (candidates from the whole-program by_name),
    // the override would hijack the binding to the unrelated-module homonym.
    let arena = Arc::new(TypeArena::new());
    let int_ty = arena.primitive(crate::type_checker::core::types::PrimKind::Int);
    let (pf, sym_ids, mut infos) = foo_overloads(&arena, vec![int_ty], vec![int_ty, int_ty]);
    infos[1].file_path = Arc::from("src/other.ts");
    let lookup = OverloadLookup::new(infos);

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", &TYPESCRIPT_PROFILE);
    let engine = Engine::build_with_hooks(
        std::slice::from_ref(&pf),
        &sym_ids,
        profiles,
        FxHashMap::default(),
        &lookup,
        arena.clone(),
    );

    let source = dummy_source();
    let r = bare_call_ref(2);
    let rc = ref_ctx_for(&r, &source);
    let fc = file_ctx_ts("src/f.ts");

    let resolution = engine.resolve(&rc, &fc, &lookup).expect("bare foo resolves");
    assert_eq!(
        resolution.target_symbol_id, 1,
        "out-of-scope 2-arg homonym must NOT hijack the in-scope first-match"
    );
    assert_ne!(resolution.strategy, "bare_overload_arg_typed");
}

#[test]
fn engine_infer_yield_returns_class_typeid_for_instantiates() {
    let pf = ts_parsed_file("src/u.ts", "export class Foo {}");
    let sym_ids = deterministic_ids(&pf);
    let lookup = EmptyLookup::from(&pf, &sym_ids);

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", &DEFAULT_PROFILE);

    let mut engine = Engine::build(std::slice::from_ref(&pf), &sym_ids, profiles, &lookup);
    let r = ExtractedRef { is_import_binding: false, is_reexport: false,
        source_symbol_index: 0,
        target_name: "Foo".to_string(),
        kind: EdgeKind::Instantiates,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    };
    let inferred = engine
        .infer_yield(&r, None, "typescript")
        .expect("Instantiates fallback interns Foo");
    assert!(matches!(
        engine.arena().get(inferred),
        crate::type_checker::core::types::Type::Class(q) if q == "Foo"
    ));
}
