use super::reduce_to_contract;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility,
};

fn make_pf(symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: "ext:pkg/unit.pas".to_string(),
        language: "pascal".to_string(),
        content_hash: "h".to_string(),
        size: 1024,
        line_count: 10,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

fn sym(name: &str, kind: SymbolKind, parent: Option<usize>) -> ExtractedSymbol {
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
        parent_index: parent,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn make_ref(source_idx: usize, target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name: target.to_string(),
        kind,
        line: 3,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 42,
        call_args: Vec::new(),
    }
}

#[test]
fn body_locals_drop_and_contract_symbols_survive() {
    // 0 class / 1 method(0) / 2 local var(1) / 3 field(0)
    let mut pf = make_pf(
        vec![
            sym("TFoo", SymbolKind::Class, None),
            sym("DoWork", SymbolKind::Method, Some(0)),
            sym("tmp", SymbolKind::Variable, Some(1)),
            sym("FCount", SymbolKind::Field, Some(0)),
        ],
        vec![],
    );
    reduce_to_contract(&mut pf);

    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["TFoo", "DoWork", "FCount"]);
    assert_eq!(
        pf.symbols[1].parent_index,
        Some(0),
        "method's parent remapped"
    );
    assert_eq!(
        pf.symbols[2].parent_index,
        Some(0),
        "field's parent remapped past the dropped local"
    );
}

#[test]
fn forward_impl_owners_cannot_leak_function_body_members_into_contracts() {
    let mut pf = make_pf(
        vec![
            sym("outer", SymbolKind::Function, None),
            sym("save", SymbolKind::Method, Some(3)),
            sym("arg", SymbolKind::Parameter, Some(1)),
            sym("Model", SymbolKind::Struct, Some(0)),
        ],
        vec![make_ref(1, "Other", EdgeKind::TypeRef)],
    );
    reduce_to_contract(&mut pf);
    assert_eq!(
        pf.symbols
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["outer"]
    );
    assert!(pf.refs.is_empty());
}

#[test]
fn forward_external_type_owners_keep_their_contract_members_and_parameters() {
    let mut pf = make_pf(
        vec![
            sym("save", SymbolKind::Method, Some(2)),
            sym("arg", SymbolKind::Parameter, Some(0)),
            sym("Model", SymbolKind::Struct, None),
            sym("local", SymbolKind::Variable, Some(0)),
        ],
        vec![],
    );
    reduce_to_contract(&mut pf);
    assert_eq!(pf.symbols.len(), 3);
    assert_eq!(pf.symbols[0].parent_index, Some(2));
    assert_eq!(pf.symbols[1].parent_index, Some(0));
}

#[test]
fn cyclic_and_dangling_parent_rows_do_not_panic_or_attest_contracts() {
    let mut pf = make_pf(
        vec![
            sym("CycleA", SymbolKind::Class, Some(1)),
            sym("CycleB", SymbolKind::Class, Some(0)),
            sym("Dangling", SymbolKind::Parameter, Some(99)),
            sym("Valid", SymbolKind::Class, None),
        ],
        vec![],
    );
    reduce_to_contract(&mut pf);
    assert_eq!(
        pf.symbols
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["Valid"]
    );
}

#[test]
fn rust_forward_local_impls_are_removed_before_cache_hydration() {
    let source = "fn outer() { use foreign::Other; impl Model { fn save() {} } struct Model; } pub struct Public; impl Public { pub fn run() {} }";
    let extracted = crate::languages::rust_lang::extract::extract(source);
    let mut pf = make_pf(extracted.symbols, extracted.refs);
    pf.language = "rust".into();
    pf.content = Some(source.into());
    reduce_to_contract(&mut pf);
    assert!(!pf
        .symbols
        .iter()
        .any(|s| s.name == "Model" || s.name == "save"));
    let owner = pf.symbols.iter().position(|s| s.name == "Public").unwrap();
    let member = pf.symbols.iter().position(|s| s.name == "run").unwrap();
    assert_eq!(pf.symbols[member].parent_index, Some(owner));
    let arena = crate::type_checker::core::types::TypeArena::new();
    let payload = super::super::external_parse_payload::CachedParse::from_parsed(&pf, &arena);
    let payload = serde_json::to_string(&payload).unwrap();
    let payload: super::super::external_parse_payload::CachedParse =
        serde_json::from_str(&payload).unwrap();
    let hydrated = payload.into_parsed(&arena, &pf.path, &pf.content_hash, pf.size, None);
    assert_eq!(hydrated.symbols.len(), pf.symbols.len());
    assert_eq!(hydrated.symbols[member].parent_index, Some(owner));
}

#[test]
fn parameters_survive_on_contract_callables_but_not_nested_closures() {
    // 0 fn / 1 param(0) / 2 nested fn(0) / 3 param-of-nested(2)
    let mut pf = make_pf(
        vec![
            sym("Top", SymbolKind::Function, None),
            sym("arg", SymbolKind::Parameter, Some(0)),
            sym("closure", SymbolKind::Function, Some(0)),
            sym("inner_arg", SymbolKind::Parameter, Some(2)),
        ],
        vec![],
    );
    reduce_to_contract(&mut pf);

    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["Top", "arg"],
        "closure and its parameter are body detail"
    );
}

#[test]
fn refs_keep_contract_kinds_from_surviving_sources_and_remap_indices() {
    // 0 class / 1 method(0) / 2 local(1)
    let mut pf = make_pf(
        vec![
            sym("TFoo", SymbolKind::Class, None),
            sym("DoWork", SymbolKind::Method, Some(0)),
            sym("tmp", SymbolKind::Variable, Some(1)),
        ],
        vec![
            make_ref(0, "TBase", EdgeKind::Inherits),
            make_ref(1, "TResult", EdgeKind::TypeRef),
            make_ref(1, "Helper", EdgeKind::Calls),
            make_ref(2, "TLocalType", EdgeKind::TypeRef),
        ],
    );
    pf.ref_origin_languages = vec![None, None, Some("x".into()), None];
    reduce_to_contract(&mut pf);

    let kept: Vec<(&str, EdgeKind)> = pf
        .refs
        .iter()
        .map(|r| (r.target_name.as_str(), r.kind))
        .collect();
    assert_eq!(
        kept,
        [
            ("TBase", EdgeKind::Inherits),
            ("TResult", EdgeKind::TypeRef)
        ]
    );
    assert_eq!(
        pf.refs[1].source_symbol_index, 1,
        "method index unchanged here"
    );
    assert_eq!(
        pf.ref_origin_languages.len(),
        2,
        "parallel ref vec sliced in step"
    );
}

#[test]
fn body_extras_clear_and_import_refs_survive() {
    let mut pf = make_pf(
        vec![sym("unit", SymbolKind::Module, None)],
        vec![make_ref(0, "SysUtils", EdgeKind::Imports)],
    );
    pf.content = Some("raw".into());
    reduce_to_contract(&mut pf);

    assert_eq!(pf.refs.len(), 1, "imports/re-exports are contract");
    assert_eq!(
        pf.content.as_deref(),
        Some("raw"),
        "content lifecycle belongs to the caller"
    );
    assert!(pf.routes.is_empty() && pf.db_sets.is_empty());
}
