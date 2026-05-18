// =============================================================================
// type_checker/engine_tests.rs — Engine façade end-to-end tests.
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolInfo, SymbolLookup};
use crate::languages::typescript::extract;
use crate::type_checker::core::{SymbolIdMap, TypeArena};
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
    let r = ExtractedRef {
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
            type_arg_ids: Vec::new(),
        }],
    };
    let source = dummy_source();
    let r = ExtractedRef {
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
fn engine_resolve_walks_single_segment_chain_to_self_yielding_class() {
    // The TS extractor emits a Class for `export class User {}`; engine.build
    // populates SymbolTypeMap's self-yield reverse index; engine.resolve on a
    // bare TypeAccess chain to User returns the class's sym id.
    let pf = ts_parsed_file("src/u.ts", "export class User {}");
    let sym_ids = deterministic_ids(&pf);
    let lookup = EmptyLookup::from(&pf, &sym_ids);

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", &DEFAULT_PROFILE);

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
            type_arg_ids: Vec::new(),
        }],
    };
    let source = dummy_source();
    let r = ExtractedRef {
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
    assert_eq!(resolution.resolved_yield_type.as_deref(), Some("User"));
    assert_eq!(resolution.strategy, "engine_chain_root");
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

#[test]
fn engine_infer_yield_returns_class_typeid_for_instantiates() {
    let pf = ts_parsed_file("src/u.ts", "export class Foo {}");
    let sym_ids = deterministic_ids(&pf);
    let lookup = EmptyLookup::from(&pf, &sym_ids);

    let mut profiles: FxHashMap<&'static str, &LanguageProfile> = FxHashMap::default();
    profiles.insert("typescript", &DEFAULT_PROFILE);

    let mut engine = Engine::build(std::slice::from_ref(&pf), &sym_ids, profiles, &lookup);
    let r = ExtractedRef {
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
