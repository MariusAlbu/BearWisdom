use super::*;
use tree_sitter::{Query, QueryCursor, StreamingIterator};

#[test]
fn incomplete_assignment_query_cannot_mutate_existing_flow_metadata() {
    let source = "const value = make();";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let cfg = FlowConfig {
        strategy_prefix: "ts",
        assignment_query: "(identifier) @rhs",
        type_guard_query: "",
        discriminant_guard_query: "",
        type_args_query: "",
        literal_type_kinds: &[],
    };
    let mut symbols = Vec::new();
    let mut meta = FlowMeta::default();
    meta.flow_binding_lhs.insert(9, 4);
    run_assignment_query(
        &tree.root_node(),
        source.as_bytes(),
        &cfg,
        &mut symbols,
        &[],
        &mut meta,
        BindingSymbols::Synthesize,
    );
    assert!(symbols.is_empty());
    assert_eq!(meta.flow_binding_lhs.len(), 1);
    assert_eq!(meta.flow_binding_lhs.get(&9), Some(&4));
    assert!(meta.flow_binding_decl_type.is_empty());
}

#[test]
fn flat_array_binding_index_preserves_elisions() {
    let source = "const [, middle, last] = tuple();";
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(source, None).unwrap();
    let query = Query::new(&language, "(array_pattern (identifier) @bind)").unwrap();
    let mut cursor = QueryCursor::new();
    let mut bindings = cursor.matches(&query, tree.root_node(), source.as_bytes());
    let mut positions = Vec::new();
    while let Some(m) = bindings.next() {
        for capture in m.captures {
            positions.push(positional_pattern(capture.node));
        }
    }

    assert_eq!(
        positions,
        vec![PositionalPattern::Slot(1), PositionalPattern::Slot(2)]
    );
}

#[test]
fn non_direct_array_elements_fence_every_positional_binding() {
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let query = Query::new(&language, "(array_pattern (identifier) @bind)").unwrap();
    for source in [
        "const [pair(1, 2), reset] = tuple();",
        "const [head = fallback, reset] = tuple();",
        "const [head, ...rest] = tuple();",
    ] {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let mut cursor = QueryCursor::new();
        let mut bindings = cursor.matches(&query, tree.root_node(), source.as_bytes());
        let mut positions = Vec::new();
        while let Some(m) = bindings.next() {
            for capture in m.captures {
                positions.push(positional_pattern(capture.node));
            }
        }
        assert_eq!(positions, vec![PositionalPattern::Unsupported], "{source}");
    }
}

#[test]
fn flat_tuple_binding_index_projects_direct_scala_positions_only() {
    let source = "val (key, inputs) = make()";
    let language: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(source, None).unwrap();
    let query = Query::new(&language, "(tuple_pattern (identifier) @bind)").unwrap();
    let mut cursor = QueryCursor::new();
    let mut bindings = cursor.matches(&query, tree.root_node(), source.as_bytes());
    let mut positions = Vec::new();
    while let Some(m) = bindings.next() {
        for capture in m.captures {
            positions.push(positional_pattern(capture.node));
        }
    }

    assert_eq!(
        positions,
        vec![PositionalPattern::Slot(0), PositionalPattern::Slot(1)]
    );
}

#[test]
fn non_flat_tuple_patterns_fence_every_positional_binding() {
    let source = "val ((key, input), rest) = make()";
    let language: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(source, None).unwrap();
    let query = Query::new(&language, "(tuple_pattern (identifier) @bind)").unwrap();
    let mut cursor = QueryCursor::new();
    let mut bindings = cursor.matches(&query, tree.root_node(), source.as_bytes());
    let mut positions = Vec::new();
    while let Some(m) = bindings.next() {
        for capture in m.captures {
            positions.push((
                capture.node.utf8_text(source.as_bytes()).unwrap(),
                positional_pattern(capture.node),
            ));
        }
    }
    assert_eq!(
        positions,
        vec![
            ("key", PositionalPattern::Unsupported),
            ("input", PositionalPattern::Unsupported),
            ("rest", PositionalPattern::Unsupported),
        ],
        "any nested direct element makes the complete tuple pattern unsupported"
    );
}

#[test]
fn unsupported_tuple_bindings_do_not_fall_back_to_object_keys() {
    let source = "val ((key, input), rest) = make()";
    let language: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(source, None).unwrap();
    let rest_offset = source.find("rest").unwrap() as u32;
    let make_offset = source.find("make").unwrap() as u32;
    let mut symbols = vec![ExtractedSymbol {
        name: "rest".into(),
        qualified_name: "rest".into(),
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: 0,
        end_line: 0,
        start_col: rest_offset,
        end_col: rest_offset + 4,
        byte_offset: rest_offset,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }];
    let refs = [ExtractedRef {
        source_symbol_index: 0,
        target_name: "make".into(),
        kind: crate::types::EdgeKind::Calls,
        line: 0,
        col: make_offset,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: make_offset,
        call_args: Vec::new(),
        is_import_binding: false,
        is_reexport: false,
        is_include: false,
    }];
    let mut meta = FlowMeta::default();
    run_assignment_query(
        &tree.root_node(),
        source.as_bytes(),
        &crate::languages::scala::flow::SCALA_FLOW_CONFIG,
        &mut symbols,
        &refs,
        &mut meta,
        BindingSymbols::Synthesize,
    );

    assert!(
        meta.flow_binding_destructure.is_empty(),
        "unsupported tuple patterns must not record a fallback field key: {meta:?}"
    );
}
