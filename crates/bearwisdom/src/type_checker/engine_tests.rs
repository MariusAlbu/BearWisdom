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
            call_args: Vec::new(),
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
            call_args: Vec::new(),
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
                call_args: Vec::new(),
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
                call_args: Vec::new(),
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

/// `wildcard` names a sibling function in `src/f.ts`, so the generic ladder's
/// same-file strategy would bind it. Returns the engine resolution for a bare
/// `Calls` ref to it under `profile`.
fn resolve_sibling_named(
    profile: &'static LanguageProfile,
) -> Option<Resolution> {
    let arena = Arc::new(TypeArena::new());
    let (mut pf, mut sym_ids, mut infos) = foo_overloads(&arena, Vec::new(), Vec::new());
    // Reshape the first `foo` into a single `wildcard` callable; drop the
    // second so the same-file strategy has one unambiguous target.
    pf.symbols.truncate(1);
    pf.symbols[0].name = "wildcard".to_string();
    pf.symbols[0].qualified_name = "wildcard".to_string();
    sym_ids = SymbolIdMap::default();
    sym_ids.insert(("src/f.ts".to_string(), 0), 1);
    infos.truncate(1);
    infos[0].name = "wildcard".to_string();
    infos[0].qualified_name = "wildcard".to_string();
    let lookup = SiblingLookup { sib: infos, empty: Vec::new(), empty_reexports: Vec::new() };

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", profile);
    let engine = Engine::build_with_hooks(
        std::slice::from_ref(&pf),
        &sym_ids,
        profiles,
        FxHashMap::default(),
        &lookup,
        arena.clone(),
    );

    let source = dummy_source();
    let mut r = bare_call_ref(0);
    r.target_name = "wildcard".to_string();
    let rc = ref_ctx_for(&r, &source);
    let fc = file_ctx_ts("src/f.ts");
    engine.resolve(&rc, &fc, &lookup)
}

/// Lookup that returns a single seeded sibling for both `by_name` and
/// `in_file`, so the generic same-file strategy can bind a bare ref.
struct SiblingLookup {
    sib: Vec<SymbolInfo>,
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl SymbolLookup for SiblingLookup {
    fn by_name(&self, name: &str) -> &[SymbolInfo] {
        if self.sib.first().map(|s| s.name.as_str()) == Some(name) {
            &self.sib
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
        &self.sib
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

/// Profile mirroring DEFAULT but skipping the `wildcard` builtin name.
static SKIP_WILDCARD_PROFILE: LanguageProfile = LanguageProfile {
    builtin_skip: Some(|n| n == "wildcard"),
    ..DEFAULT_PROFILE
};

#[test]
fn builtin_skip_declines_before_ladder_binds_sibling() {
    // With no builtin_skip, the same-file strategy binds the sibling `wildcard`.
    let bound = resolve_sibling_named(&DEFAULT_PROFILE)
        .expect("ladder binds the same-file sibling when nothing skips it");
    assert_eq!(bound.target_symbol_id, 1);

    // With builtin_skip recognizing `wildcard`, the engine declines outright —
    // the ladder never runs, so the sibling is NOT bound.
    assert!(
        resolve_sibling_named(&SKIP_WILDCARD_PROFILE).is_none(),
        "builtin_skip must decline the ref before the ladder binds a homonym"
    );
}

#[test]
fn builtin_skip_none_leaves_other_targets_resolvable() {
    // The skip predicate matches ONLY `wildcard`; a sibling under any other
    // name still resolves through the ladder under the same profile. Proves the
    // gate is per-target, not a blanket decline.
    let arena = Arc::new(TypeArena::new());
    let (mut pf, mut sym_ids, mut infos) = foo_overloads(&arena, Vec::new(), Vec::new());
    pf.symbols.truncate(1);
    pf.symbols[0].name = "helper".to_string();
    pf.symbols[0].qualified_name = "helper".to_string();
    sym_ids = SymbolIdMap::default();
    sym_ids.insert(("src/f.ts".to_string(), 0), 1);
    infos.truncate(1);
    infos[0].name = "helper".to_string();
    infos[0].qualified_name = "helper".to_string();
    let lookup = SiblingLookup { sib: infos, empty: Vec::new(), empty_reexports: Vec::new() };

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", &SKIP_WILDCARD_PROFILE);
    let engine = Engine::build_with_hooks(
        std::slice::from_ref(&pf),
        &sym_ids,
        profiles,
        FxHashMap::default(),
        &lookup,
        arena.clone(),
    );

    let source = dummy_source();
    let mut r = bare_call_ref(0);
    r.target_name = "helper".to_string();
    let rc = ref_ctx_for(&r, &source);
    let fc = file_ctx_ts("src/f.ts");
    let resolution = engine
        .resolve(&rc, &fc, &lookup)
        .expect("non-builtin sibling still resolves under a builtin_skip profile");
    assert_eq!(resolution.target_symbol_id, 1);
}

/// Resolve a bare `target` call against a single same-file sibling of that
/// name, under `profile`, in a file whose context carries `file_namespace`.
fn resolve_sibling_in_namespace(
    profile: &'static LanguageProfile,
    target: &str,
    file_namespace: Option<&str>,
) -> Option<Resolution> {
    let arena = Arc::new(TypeArena::new());
    let (mut pf, mut sym_ids, mut infos) = foo_overloads(&arena, Vec::new(), Vec::new());
    pf.symbols.truncate(1);
    pf.symbols[0].name = target.to_string();
    pf.symbols[0].qualified_name = target.to_string();
    sym_ids = SymbolIdMap::default();
    sym_ids.insert(("src/f.ts".to_string(), 0), 1);
    infos.truncate(1);
    infos[0].name = target.to_string();
    infos[0].qualified_name = target.to_string();
    let lookup = SiblingLookup { sib: infos, empty: Vec::new(), empty_reexports: Vec::new() };

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", profile);
    let engine = Engine::build_with_hooks(
        std::slice::from_ref(&pf),
        &sym_ids,
        profiles,
        FxHashMap::default(),
        &lookup,
        arena.clone(),
    );

    let source = dummy_source();
    let mut r = bare_call_ref(0);
    r.target_name = target.to_string();
    let rc = ref_ctx_for(&r, &source);
    let mut fc = file_ctx_ts("src/f.ts");
    fc.file_namespace = file_namespace.map(|s| s.to_string());
    engine.resolve(&rc, &fc, &lookup)
}

/// Profile mirroring DEFAULT but declining `reserved` only inside files whose
/// namespace is `ns-sentinel` — the two-key namespace-gated decline.
static NS_DECLINE_PROFILE: LanguageProfile = LanguageProfile {
    namespace_decline: Some(
        crate::type_checker::profile::language_profile::NamespaceDecline {
            file_namespace: "ns-sentinel",
            is_reserved: |n| n == "reserved",
        },
    ),
    ..DEFAULT_PROFILE
};

#[test]
fn namespace_decline_gates_on_both_keys() {
    // Reserved name + armed namespace → declines before the ladder, so the
    // same-file sibling is NOT bound.
    assert!(
        resolve_sibling_in_namespace(&NS_DECLINE_PROFILE, "reserved", Some("ns-sentinel"))
            .is_none(),
        "namespace_decline must decline when both the namespace and the name match"
    );

    // Reserved name but the file is in a different namespace → the second key
    // is absent, so the ladder still binds the sibling.
    let bound =
        resolve_sibling_in_namespace(&NS_DECLINE_PROFILE, "reserved", Some("other-ns"))
            .expect("a different namespace must not arm the decline");
    assert_eq!(bound.target_symbol_id, 1);

    // Armed namespace but a non-reserved name → the first key is absent, so the
    // ladder still binds the sibling. Proves the gate is per-target.
    let bound =
        resolve_sibling_in_namespace(&NS_DECLINE_PROFILE, "ordinary", Some("ns-sentinel"))
            .expect("a non-reserved name in the armed namespace must still resolve");
    assert_eq!(bound.target_symbol_id, 1);
}

/// Resolve a `sep`-qualified `target` whose sibling is named identically (so
/// the same-file strategy WOULD bind it) against `profile`, in a file whose
/// import set declares `import_module`.
fn resolve_qualified_sibling_with_import(
    profile: &'static LanguageProfile,
    target: &str,
    import_module: &str,
) -> Option<Resolution> {
    let arena = Arc::new(TypeArena::new());
    let (mut pf, mut sym_ids, mut infos) = foo_overloads(&arena, Vec::new(), Vec::new());
    pf.symbols.truncate(1);
    pf.symbols[0].name = target.to_string();
    pf.symbols[0].qualified_name = target.to_string();
    sym_ids = SymbolIdMap::default();
    sym_ids.insert(("src/f.ts".to_string(), 0), 1);
    infos.truncate(1);
    infos[0].name = target.to_string();
    infos[0].qualified_name = target.to_string();
    let lookup = SiblingLookup { sib: infos, empty: Vec::new(), empty_reexports: Vec::new() };

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", profile);
    let engine = Engine::build_with_hooks(
        std::slice::from_ref(&pf),
        &sym_ids,
        profiles,
        FxHashMap::default(),
        &lookup,
        arena.clone(),
    );

    let source = dummy_source();
    let mut r = bare_call_ref(0);
    r.target_name = target.to_string();
    let rc = ref_ctx_for(&r, &source);
    let mut fc = file_ctx_ts("src/f.ts");
    fc.imports.push(crate::indexer::resolve::engine::ImportEntry {
        imported_name: import_module.to_string(),
        module_path: Some(import_module.to_string()),
        alias: None,
        is_wildcard: true,
    });
    engine.resolve(&rc, &fc, &lookup)
}

/// Profile mirroring DEFAULT but with a `::` separator and the import-prefix
/// decline armed — the Puppet shape.
static IMPORT_PREFIX_DECLINE_PROFILE: LanguageProfile = LanguageProfile {
    qname_separator: "::",
    decline_qualified_when_prefix_imported: true,
    ..DEFAULT_PROFILE
};

/// Control: same `::` separator, decline OFF.
static IMPORT_PREFIX_NO_DECLINE_PROFILE: LanguageProfile = LanguageProfile {
    qname_separator: "::",
    decline_qualified_when_prefix_imported: false,
    ..DEFAULT_PROFILE
};

#[test]
fn import_prefix_decline_gates_on_leading_segment_vs_imports() {
    // Decline armed + target's leading `::` segment names a declared import →
    // declines before the ladder, so the same-file sibling is NOT bound.
    assert!(
        resolve_qualified_sibling_with_import(
            &IMPORT_PREFIX_DECLINE_PROFILE,
            "apache::config",
            "apache",
        )
        .is_none(),
        "a qualified target under an imported module must decline before binding a local"
    );

    // Control: decline OFF → the same-file strategy binds the sibling.
    let bound = resolve_qualified_sibling_with_import(
        &IMPORT_PREFIX_NO_DECLINE_PROFILE,
        "apache::config",
        "apache",
    )
    .expect("with the decline off the ladder binds the same-file sibling");
    assert_eq!(bound.target_symbol_id, 1);

    // Decline armed but the leading segment is NOT in the import set → the gate
    // is absent, so the ladder still binds. Proves the gate keys on the import
    // set, not on the mere presence of the separator.
    let bound = resolve_qualified_sibling_with_import(
        &IMPORT_PREFIX_DECLINE_PROFILE,
        "nginx::config",
        "apache",
    )
    .expect("a qualified target whose head is not imported must still resolve");
    assert_eq!(bound.target_symbol_id, 1);
}

// =============================================================================
// LANG-CPP-1 — argument-dependent lookup (ADL).
// =============================================================================

/// Lookup double for the ADL test. A free function `swap` is keyed ONLY under
/// the qname `mylib.swap` (reachable via `by_qualified_name`); `by_name("swap")`
/// is empty so the regular bare-name ladder declines. Two call arguments `a`/`b`
/// both have local type `mylib.Widget`, so their declaring namespace `mylib`
/// supplies the `swap` candidate.
struct AdlLookup {
    swap: Vec<SymbolInfo>,
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl AdlLookup {
    fn new() -> Self {
        let file_path: Arc<str> = Arc::from("src/lib.cpp");
        Self {
            swap: vec![SymbolInfo {
                id: 7,
                name: "swap".to_string(),
                qualified_name: "mylib.swap".to_string(),
                kind: "function".to_string(),
                visibility: Some("public".to_string()),
                file_path,
                scope_path: Some("mylib".to_string()),
                package_id: None,
                signature: None,
            }],
            empty: Vec::new(),
            empty_reexports: Vec::new(),
        }
    }
}

impl SymbolLookup for AdlLookup {
    // `swap` is NOT in bare-name scope — the regular ladder finds nothing and
    // declines, so ADL is strictly the fallback.
    fn by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    // The argument's declaring namespace supplies the candidate by qname.
    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo> {
        if qname == "mylib.swap" {
            self.swap.first()
        } else {
            None
        }
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
    // The arguments' declared type — the source of the ADL namespace.
    fn local_type(&self, name: &str) -> Option<String> {
        match name {
            "a" | "b" => Some("mylib.Widget".to_string()),
            _ => None,
        }
    }
}

/// A bare `Calls` ref `swap(a, b)` whose two arguments are identifiers `a`/`b`.
fn adl_swap_ref() -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: "swap".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: vec![
            crate::types::CallArg::Ident("a".to_string()),
            crate::types::CallArg::Ident("b".to_string()),
        ],
    }
}

/// Build an engine over the ADL lookup under `language`/`profile` and resolve
/// the bare `swap(a, b)` ref.
fn resolve_adl_swap(
    language: &'static str,
    profile: &'static LanguageProfile,
) -> Option<Resolution> {
    let arena = Arc::new(TypeArena::new());
    // No ParsedFile symbols are needed — the `swap` target lives only in the
    // lookup double, reachable by qname. An empty parsed file gives the engine
    // a registered profile under `language`.
    let pf = ParsedFile {
        path: "src/lib.cpp".to_string(),
        language: language.to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
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
    let sym_ids = SymbolIdMap::default();
    let lookup = AdlLookup::new();

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert(language, profile);
    let engine = Engine::build_with_hooks(
        std::slice::from_ref(&pf),
        &sym_ids,
        profiles,
        FxHashMap::default(),
        &lookup,
        arena.clone(),
    );

    let source = dummy_source();
    let r = adl_swap_ref();
    let rc = ref_ctx_for(&r, &source);
    let fc = FileContext {
        file_path: "src/lib.cpp".to_string(),
        language: language.to_string(),
        imports: Vec::new(),
        file_namespace: None,
    };
    engine.resolve(&rc, &fc, &lookup)
}

#[test]
fn adl_resolves_bare_call_via_argument_namespace() {
    // C++ profile opts into ADL: a bare `swap(a, b)` the ladder declined
    // resolves to `mylib.swap` because argument `a`'s type `mylib.Widget`
    // declares the namespace `mylib`, which owns `swap`.
    let resolution = resolve_adl_swap("c", &crate::languages::c_lang::C_LANG_PROFILE)
        .expect("ADL binds swap via the argument's declaring namespace");
    assert_eq!(
        resolution.target_symbol_id, 7,
        "ADL must bind the namespace-mate `mylib.swap`"
    );
    assert_eq!(resolution.strategy, "engine_adl");
}

#[test]
fn adl_gated_off_declines_same_ref() {
    // Same scenario, but under a profile with `argument_dependent_lookup`
    // false (TypeScript): the probe never runs, so the ref stays unresolved —
    // proving ADL is gated by the profile axis, not unconditional.
    assert!(
        resolve_adl_swap("typescript", &TYPESCRIPT_PROFILE).is_none(),
        "ADL must not fire for a profile that hasn't opted in"
    );
}

// ---------------------------------------------------------------------------
// EXT-2 Phase 0 — discover the true external-chain frontier.
//
// `repo.get().greet()` where `repo: Repository` (an ext: class), `get` returns
// an ext: `User`, and `greet` is a `User` method. The question this answers:
// does an external class-method chain resolve end to end through the by-qname
// fallback + external return-type yield, or does it break — and where?
// ---------------------------------------------------------------------------

fn ext2_pf(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        content: None,
        has_errors: false,
        flow: Default::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn ext2_sym(name: &str, qname: &str, kind: SymbolKind, scope: Option<&str>, sig: Option<&str>) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: sig.map(str::to_string),
        doc_comment: None,
        scope_path: scope.map(str::to_string),
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn ext2_seg(name: &str, kind: SegmentKind, declared: Option<&str>, is_call: bool) -> ChainSegment {
    ChainSegment {
        name: name.to_string(),
        node_kind: "x".to_string(),
        kind,
        declared_type: declared.map(str::to_string),
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }
}

#[test]
fn augment_engine_resolves_identically_to_full_build() {
    // P1 gate: an Engine built over [A] then augmented with [B] must resolve a
    // B-dependent chain identically to an Engine built over [A, B] in one shot.
    // `f.bar().qux()` needs Foo.bar(): Baz (file A) and Baz.qux() (file B).
    use crate::indexer::resolve::engine::{build_scope_chain, SymbolIndex};
    use crate::type_checker::core::SymbolIdMap;
    use std::collections::HashMap;

    let chain = MemberChain {
        segments: vec![
            ext2_seg("f", SegmentKind::Identifier, Some("Foo"), false),
            ext2_seg("bar", SegmentKind::Property, None, true),
            ext2_seg("qux", SegmentKind::Property, None, true),
        ],
    };
    let chain_ref = ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 1,
        target_name: "f.bar.qux".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: Some(chain),
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };
    let file_a = ext2_pf(
        "a.ts",
        vec![
            ext2_sym("Foo", "Foo", SymbolKind::Class, None, Some("class Foo")),
            ext2_sym("use", "use", SymbolKind::Function, None, None),
            ext2_sym("bar", "Foo.bar", SymbolKind::Method, Some("Foo"), Some("bar(): Baz")),
        ],
        vec![chain_ref],
    );
    let file_b = ext2_pf(
        "b.ts",
        vec![
            ext2_sym("Baz", "Baz", SymbolKind::Class, None, Some("class Baz")),
            ext2_sym("qux", "Baz.qux", SymbolKind::Method, Some("Baz"), Some("qux(): void")),
        ],
        vec![],
    );

    let files = vec![file_a, file_b];
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    let mut next = 1i64;
    for p in &files {
        for s in &p.symbols {
            id_map.insert((p.path.clone(), s.qualified_name.clone()), next);
            next += 1;
        }
    }
    let index = SymbolIndex::build(&files, &id_map);
    let mut eng = SymbolIdMap::default();
    for p in &files {
        for (i, s) in p.symbols.iter().enumerate() {
            if let Some(&id) = id_map.get(&(p.path.clone(), s.qualified_name.clone())) {
                eng.insert((p.path.clone(), i), id);
            }
        }
    }

    let fc = FileContext {
        file_path: "a.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let resolve_with = |engine: &Engine| {
        let r = &files[0].refs[0];
        let rc = RefContext {
            extracted_ref: r,
            source_symbol: &files[0].symbols[1],
            scope_chain: build_scope_chain(files[0].symbols[1].scope_path.as_deref()),
            file_package_id: None,
        };
        engine.resolve(&rc, &fc, &index).map(|res| res.target_symbol_id)
    };
    // The Engine's own members are what `augment` mutates; resolution is masked
    // by the SymbolIndex by-qname fallback (built over the full set), so compare
    // the engine's internal member set directly to prove augment == rebuild.
    let baz_members = |engine: &Engine| -> Vec<(String, i64)> {
        let baz = engine.arena().class("Baz");
        let mut v: Vec<(String, i64)> = engine
            .members
            .direct_of(baz)
            .iter()
            .map(|m| (m.name.clone(), m.id))
            .collect();
        v.sort();
        v
    };

    // One-shot build over both files.
    let full = Engine::build_from_registry(&files, &eng, &index, index.type_arena_arc());

    // Build over A only, then augment with B.
    let a_only = std::slice::from_ref(&files[0]);
    let mut augmented = Engine::build_from_registry(a_only, &eng, &index, index.type_arena_arc());
    let before = baz_members(&augmented);
    augmented.augment(&files, std::slice::from_ref(&files[1]), &eng, &index);
    let after = baz_members(&augmented);

    let qux_id = id_map[&("b.ts".to_string(), "Baz.qux".to_string())];
    assert!(
        before.is_empty(),
        "before augment, Baz's members are absent from the A-only engine: {before:?}"
    );
    assert_eq!(
        after,
        baz_members(&full),
        "augment([B]) must produce Baz's member set identical to a full build over [A, B]"
    );
    assert_eq!(after, vec![("qux".to_string(), qux_id)], "Baz gains its qux member");
    assert_eq!(
        resolve_with(&augmented),
        resolve_with(&full),
        "end-to-end resolution is identical after augment"
    );
}

#[test]
fn ext2_external_class_method_chain_resolves_end_to_end() {
    use crate::indexer::resolve::engine::{build_scope_chain, SymbolIndex};
    use crate::type_checker::core::SymbolIdMap;
    use std::collections::HashMap;

    // ext: dep — Repository.get(): User and a User.greet() method.
    let ext = ext2_pf(
        "ext:ts:orm/index.d.ts",
        vec![
            ext2_sym("Repository", "Repository", SymbolKind::Class, None, Some("class Repository")),
            ext2_sym("get", "Repository.get", SymbolKind::Method, Some("Repository"), Some("get(): User")),
            ext2_sym("User", "User", SymbolKind::Class, None, Some("class User")),
            ext2_sym("greet", "User.greet", SymbolKind::Method, Some("User"), Some("greet(): void")),
        ],
        vec![],
    );

    // app — useRepo() containing the chain `repo.get().greet()`; the root is
    // typed Repository via a declared_type assertion so the test isolates the
    // external member-walk + return-type yield from root-typing machinery.
    let chain = MemberChain {
        segments: vec![
            ext2_seg("repo", SegmentKind::Identifier, Some("Repository"), false),
            ext2_seg("get", SegmentKind::Property, None, true),
            ext2_seg("greet", SegmentKind::Property, None, true),
        ],
    };
    let chain_ref = ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: "repo.get.greet".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: Some(chain),
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };
    let app = ext2_pf(
        "app.ts",
        vec![ext2_sym("useRepo", "useRepo", SymbolKind::Function, None, None)],
        vec![chain_ref],
    );

    let files = vec![ext, app];
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    let mut next = 1i64;
    for p in &files {
        for s in &p.symbols {
            id_map.insert((p.path.clone(), s.qualified_name.clone()), next);
            next += 1;
        }
    }
    let index = SymbolIndex::build(&files, &id_map);
    let mut eng = SymbolIdMap::default();
    for p in &files {
        for (i, s) in p.symbols.iter().enumerate() {
            if let Some(&id) = id_map.get(&(p.path.clone(), s.qualified_name.clone())) {
                eng.insert((p.path.clone(), i), id);
            }
        }
    }
    let engine = Engine::build_from_registry(&files, &eng, &index, index.type_arena_arc());

    let app_file = &files[1];
    let r = &app_file.refs[0];
    let fc = FileContext {
        file_path: "app.ts".to_string(),
        language: "typescript".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let rc = RefContext {
        extracted_ref: r,
        source_symbol: &app_file.symbols[0],
        scope_chain: build_scope_chain(app_file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let resolution = engine.resolve(&rc, &fc, &index);
    let greet_id = id_map[&("ext:ts:orm/index.d.ts".to_string(), "User.greet".to_string())];
    let misses = index.take_chain_misses();
    assert_eq!(
        resolution.map(|r| r.target_symbol_id),
        Some(greet_id),
        "repo.get().greet() should bind greet on the external return type User; chain misses: {:?}",
        misses.iter().map(|m| (&m.current_type, &m.target_name)).collect::<Vec<_>>()
    );
}
