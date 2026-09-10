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
            positions.push(array_pattern_index(capture.node));
        }
    }

    assert_eq!(positions, vec![Some(1), Some(2)]);
}

#[test]
fn flat_array_binding_index_ignores_commas_inside_earlier_elements() {
    let source = "const [pair(1, 2), reset] = tuple();";
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
            positions.push(array_pattern_index(capture.node));
        }
    }

    assert_eq!(positions, vec![Some(1)]);
}
