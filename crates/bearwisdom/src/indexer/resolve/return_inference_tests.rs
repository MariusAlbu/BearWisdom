use super::*;
use crate::type_checker::core::types::TypeId;
use crate::types::{ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility};

fn fn_sym(qname: &str, ret: Option<TypeId>) -> ExtractedSymbol {
    ExtractedSymbol {
        name: qname.rsplit('.').next().unwrap_or(qname).to_string(),
        qualified_name: qname.to_string(),
        kind: SymbolKind::Function,
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
        return_type: ret,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn file(path: &str, syms: Vec<ExtractedSymbol>, return_lhs: &[(usize, usize)]) -> ParsedFile {
    let mut flow = FlowMeta::default();
    for &(ref_idx, fn_idx) in return_lhs {
        flow.flow_return_lhs.insert(ref_idx, fn_idx);
    }
    ParsedFile {
        path: path.to_string(),
        language: "typescript".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: syms,
        refs: vec![],
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow,
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

#[test]
fn groups_return_refs_by_function_skipping_annotated_and_external() {
    let arena = crate::type_checker::core::types::TypeArena::new();
    let foo: TypeId = arena.class("Foo");
    let parsed = vec![
        // Internal file: fn 0 `makeRepo` is un-annotated with two return refs (3, 1);
        // fn 1 `typed` has a declared return → excluded.
        file(
            "app.ts",
            vec![fn_sym("makeRepo", None), fn_sym("typed", Some(foo))],
            &[(3, 0), (1, 0), (5, 1)],
        ),
        // External file: skipped wholesale even though its fn is un-annotated.
        file("ext:ts:lib.d.ts", vec![fn_sym("ext.make", None)], &[(0, 0)]),
    ];

    let got = build_function_returns(&parsed);

    assert_eq!(got.len(), 1, "only the un-annotated internal function appears");
    let fr = &got[0];
    assert_eq!(fr.qname, "makeRepo");
    assert_eq!(fr.file, 0);
    assert_eq!(fr.fn_idx, 0);
    assert_eq!(fr.return_refs, vec![1, 3], "return refs sorted ascending");
}

#[test]
fn function_without_return_refs_does_not_appear() {
    let parsed = vec![file("v.ts", vec![fn_sym("voidFn", None)], &[])];
    assert!(build_function_returns(&parsed).is_empty());
}
