use super::*;
use crate::type_checker::core::types::{GenericParamData, Type};

/// A ParsedFile with every carried field populated, plus a source arena whose
/// TypeIds back the symbol/segment type fields.
fn sample() -> (ParsedFile, TypeArena) {
    let arena = TypeArena::new();
    let vec_doc = arena.intern_type_str("Vec<tantivy.Document>");
    let u64_ty = arena.class("u64");
    let t_param = arena.intern_generic(GenericParamData {
        name: "T".into(),
        owner_symbol_index: 0,
        bound: Some(arena.class("Ord")),
    });
    let generic_t = arena.intern(Type::Generic { param: t_param });

    let field_sym = ExtractedSymbol {
        name: "Index".into(),
        qualified_name: "windows.SYMBOL_INFO.Index".into(),
        kind: SymbolKind::Field,
        visibility: Some(Visibility::Public),
        start_line: 10,
        end_line: 10,
        start_col: 4,
        end_col: 9,
        byte_offset: 100,
        signature: Some("pub Index: u64".into()),
        doc_comment: Some("doc".into()),
        scope_path: Some("windows.SYMBOL_INFO".into()),
        parent_index: Some(1),
        declared_type: Some(u64_ty),
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    };
    let fn_sym = ExtractedSymbol {
        name: "search".into(),
        qualified_name: "tantivy.search".into(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 20,
        end_line: 30,
        start_col: 0,
        end_col: 1,
        byte_offset: 400,
        signature: Some("fn search<T: Ord>(q: T) -> Vec<Document>".into()),
        doc_comment: None,
        scope_path: Some("tantivy".into()),
        parent_index: None,
        declared_type: None,
        return_type: Some(vec_doc),
        param_types: vec![generic_t],
        generic_params: vec![t_param],
    };

    let chain_ref = ExtractedRef {
        is_include: false,
        source_symbol_index: 1,
        target_name: "collect".into(),
        kind: EdgeKind::Calls,
        line: 25,
        col: 8,
        byte_offset: 520,
        module: Some("tantivy".into()),
        namespace_segments: vec!["collector".into()],
        chain: Some(MemberChain {
            segments: vec![ChainSegment {
                name: "docs".into(),
                node_kind: "identifier".into(),
                kind: SegmentKind::Identifier,
                declared_type: Some("Vec<Document>".into()),
                type_args: vec!["Document".into()],
                optional_chaining: true,
                byte_offset: 512,
                declared_type_id: Some(vec_doc),
                type_arg_ids: vec![arena.class("Document")],
                is_call: true,
                call_args: vec![CallArg::Ident("q".into())],
            }],
        }),
        call_args: vec![
            CallArg::StringLit("users".into()),
            CallArg::TaggedTemplate {
                tag: "sql".into(),
                body: "select 1".into(),
            },
        ],
        is_import_binding: false,
        is_reexport: true,
    };

    let pf = ParsedFile {
        path: "ext:idx:dep/src/lib.rs".into(),
        language: "rust".into(),
        content_hash: "abc123".into(),
        size: 2048,
        line_count: 77,
        mtime: Some(1_700_000_000),
        package_id: Some(9),
        symbols: vec![field_sym, fn_sym],
        refs: vec![chain_ref],
        routes: vec![ExtractedRoute {
            handler_symbol_index: 1,
            http_method: "GET".into(),
            template: "/api/{id}".into(),
        }],
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None, Some("rust".into())],
        ref_origin_languages: vec![Some("rust".into())],
        symbol_from_snippet: vec![false, true],
        content: Some("source text".into()),
        has_errors: true,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: vec![(
            "tantivy.Result".into(),
            AliasTarget::Intersection(vec!["A".into(), "B".into()]),
        )],
        component_selectors: vec![("nb-card".into(), "@nebular/theme.NbCardComponent".into())],
        plugin_flow_emissions: Vec::new(),
        declared_modules: vec!["virtual:pwa-register".into()],
    };
    (pf, arena)
}

/// Serde round-trip of the payload, rehydrated into a FRESH arena — the
/// warm-reindex shape, where the arena that interned the original TypeIds is
/// gone.
fn roundtrip(pf: &ParsedFile, src: &TypeArena, dst: &TypeArena) -> ParsedFile {
    let cp = CachedParse::from_parsed(pf, src);
    let json = serde_json::to_string(&cp).expect("serialize");
    let back: CachedParse = serde_json::from_str(&json).expect("deserialize");
    back.into_parsed(dst, &pf.path, &pf.content_hash, pf.size, pf.mtime)
}

#[test]
fn parsed_file_roundtrip_is_field_for_field_faithful() {
    let (pf, src) = sample();
    let dst = TypeArena::new();
    let got = roundtrip(&pf, &src, &dst);

    // Exhaustive destructure — adding a field to ParsedFile breaks this test
    // at compile time until the field is classified as carried or excluded.
    let ParsedFile {
        path,
        language,
        content_hash,
        size,
        line_count,
        mtime,
        package_id,
        symbols,
        refs,
        routes,
        db_sets,
        symbol_origin_languages,
        ref_origin_languages,
        symbol_from_snippet,
        content,
        has_errors,
        flow: _flow,
        demand_contributions,
        alias_targets,
        component_selectors,
        plugin_flow_emissions,
        declared_modules,
    } = got;

    assert_eq!(path, pf.path);
    assert_eq!(language, pf.language);
    assert_eq!(content_hash, pf.content_hash);
    assert_eq!(size, pf.size);
    assert_eq!(line_count, pf.line_count);
    assert_eq!(mtime, pf.mtime);
    assert_eq!(package_id, pf.package_id);
    assert_eq!(symbols.len(), pf.symbols.len());
    assert_eq!(refs.len(), pf.refs.len());
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].handler_symbol_index, 1);
    assert_eq!(routes[0].http_method, "GET");
    assert_eq!(routes[0].template, "/api/{id}");
    assert_eq!(symbol_origin_languages, pf.symbol_origin_languages);
    assert_eq!(ref_origin_languages, pf.ref_origin_languages);
    assert_eq!(symbol_from_snippet, pf.symbol_from_snippet);
    assert_eq!(has_errors, pf.has_errors);
    assert_eq!(alias_targets, pf.alias_targets);
    assert_eq!(component_selectors, pf.component_selectors);
    assert_eq!(declared_modules, pf.declared_modules);
    // Excluded by contract: content is re-read from disk by its consumers;
    // the remaining fields have no external-file consumer.
    assert!(content.is_none());
    assert!(db_sets.is_empty());
    assert!(demand_contributions.is_empty());
    assert!(plugin_flow_emissions.is_empty());
}

#[test]
fn symbol_roundtrip_preserves_every_field_and_type() {
    let (pf, src) = sample();
    let dst = TypeArena::new();
    let got = roundtrip(&pf, &src, &dst);

    // Exhaustive destructure — a new ExtractedSymbol field breaks this test
    // at compile time until the payload carries it.
    let ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
        start_line,
        end_line,
        start_col,
        end_col,
        byte_offset,
        signature,
        doc_comment,
        scope_path,
        parent_index,
        declared_type,
        return_type,
        param_types,
        generic_params,
    } = got.symbols[0].clone();
    let orig = &pf.symbols[0];
    assert_eq!(name, orig.name);
    assert_eq!(qualified_name, orig.qualified_name);
    assert_eq!(kind, orig.kind);
    assert_eq!(visibility, orig.visibility);
    assert_eq!(start_line, orig.start_line);
    assert_eq!(end_line, orig.end_line);
    assert_eq!(start_col, orig.start_col);
    assert_eq!(end_col, orig.end_col);
    assert_eq!(byte_offset, orig.byte_offset);
    assert_eq!(signature, orig.signature);
    assert_eq!(doc_comment, orig.doc_comment);
    assert_eq!(scope_path, orig.scope_path);
    assert_eq!(parent_index, orig.parent_index);
    assert_eq!(return_type, None);
    assert!(param_types.is_empty());
    assert!(generic_params.is_empty());
    // The field's declared type re-interns structurally.
    let declared = declared_type.expect("declared_type must survive the cache");
    assert_eq!(dst.get(declared), Type::Class("u64".into()));

    // The function symbol: return type, generic-threaded param type, and the
    // generic-params slot must all survive with identity intact.
    let f = &got.symbols[1];
    let ret = f.return_type.expect("return_type must survive the cache");
    let expected_ret = dst.intern_type_str("Vec<tantivy.Document>");
    assert_eq!(ret, expected_ret);
    assert_eq!(f.generic_params.len(), 1);
    let Type::Generic { param } = dst.get(f.param_types[0]) else {
        panic!("expected Generic param type");
    };
    assert_eq!(
        param, f.generic_params[0],
        "the param-type occurrence and the generic_params slot must share one id"
    );
    let data = dst.generic_param(param);
    assert_eq!(data.name, "T");
    assert_eq!(dst.get(data.bound.expect("bound survives")), Type::Class("Ord".into()));
}

#[test]
fn ref_roundtrip_preserves_chain_and_call_args() {
    let (pf, src) = sample();
    let dst = TypeArena::new();
    let got = roundtrip(&pf, &src, &dst);

    // Exhaustive destructure — a new ExtractedRef field breaks this test at
    // compile time until the payload carries it.
    let ExtractedRef {
        is_include: _,
        source_symbol_index,
        target_name,
        kind,
        line,
        col,
        byte_offset,
        module,
        namespace_segments,
        chain,
        call_args,
        is_import_binding,
        is_reexport,
    } = got.refs[0].clone();
    let orig = &pf.refs[0];
    assert_eq!(source_symbol_index, orig.source_symbol_index);
    assert_eq!(target_name, orig.target_name);
    assert_eq!(kind, orig.kind);
    assert_eq!(line, orig.line);
    assert_eq!(col, orig.col);
    assert_eq!(byte_offset, orig.byte_offset);
    assert_eq!(module, orig.module);
    assert_eq!(namespace_segments, orig.namespace_segments);
    assert_eq!(call_args, orig.call_args);
    assert_eq!(is_import_binding, orig.is_import_binding);
    assert_eq!(is_reexport, orig.is_reexport);

    let chain = chain.expect("chain must survive the cache");
    let orig_seg = &orig.chain.as_ref().unwrap().segments[0];
    // Exhaustive destructure — a new ChainSegment field breaks this test at
    // compile time until the payload carries it.
    let ChainSegment {
        name,
        node_kind,
        kind: seg_kind,
        declared_type,
        type_args,
        optional_chaining,
        byte_offset: seg_byte_offset,
        declared_type_id,
        type_arg_ids,
        is_call,
        call_args: seg_call_args,
    } = chain.segments[0].clone();
    assert_eq!(name, orig_seg.name);
    assert_eq!(node_kind, orig_seg.node_kind);
    assert_eq!(seg_kind, orig_seg.kind);
    assert_eq!(declared_type, orig_seg.declared_type);
    assert_eq!(type_args, orig_seg.type_args);
    assert_eq!(optional_chaining, orig_seg.optional_chaining);
    assert_eq!(seg_byte_offset, orig_seg.byte_offset);
    assert_eq!(is_call, orig_seg.is_call);
    assert_eq!(seg_call_args, orig_seg.call_args);
    let dt = declared_type_id.expect("segment declared_type_id must survive");
    assert_eq!(dt, dst.intern_type_str("Vec<tantivy.Document>"));
    assert_eq!(type_arg_ids.len(), 1);
    assert_eq!(dst.get(type_arg_ids[0]), Type::Class("Document".into()));
}
