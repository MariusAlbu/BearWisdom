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
        None,
        None,
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
            positions.push(crate::languages::typescript::flow::destructure_shape(
                capture.node,
            ));
        }
    }

    assert_eq!(
        positions,
        vec![DestructureShape::Slot(1), DestructureShape::Slot(2)]
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
                positions.push(crate::languages::typescript::flow::destructure_shape(
                    capture.node,
                ));
            }
        }
        assert_eq!(positions, vec![DestructureShape::Unsupported], "{source}");
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
            positions.push(crate::languages::scala::flow::destructure_shape(
                capture.node,
            ));
        }
    }

    assert_eq!(
        positions,
        vec![DestructureShape::Slot(0), DestructureShape::Slot(1)]
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
                crate::languages::scala::flow::destructure_shape(capture.node),
            ));
        }
    }
    assert_eq!(
        positions,
        vec![
            ("key", DestructureShape::Unsupported),
            ("input", DestructureShape::Unsupported),
            ("rest", DestructureShape::Unsupported),
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
        Some(&crate::languages::scala::ScalaPlugin),
        Some(&crate::languages::scala::flow::SCALA_CFG_KINDS),
    );

    assert!(
        meta.flow_binding_destructure.is_empty(),
        "unsupported tuple patterns must not record a fallback field key: {meta:?}"
    );
}

// ---------------------------------------------------------------------------
// Member-target assignments
// ---------------------------------------------------------------------------

/// Run the PHP extractor plus the flow pass over `source`, returning the
/// resulting metadata alongside the symbols and refs it was correlated against.
fn php_flow(source: &str) -> (FlowMeta, Vec<ExtractedSymbol>, Vec<ExtractedRef>) {
    use crate::languages::LanguagePlugin;
    let result = crate::languages::php::extract::extract(source);
    let mut symbols = result.symbols;
    let mut refs = result.refs;
    let grammar = crate::languages::php::PhpPlugin.grammar("php").unwrap();
    let cfg = crate::languages::php::PhpPlugin.flow_config().unwrap();
    let meta = crate::indexer::flow::run_flow_queries(
        source,
        &grammar,
        cfg,
        &mut symbols,
        &mut refs,
        BindingSymbols::Synthesize,
    );
    (meta, symbols, refs)
}

const MEMBER_ASSIGNMENT: &str = r#"<?php

namespace Fx;

class AssignCase
{
    protected $factory;

    protected function setUp()
    {
        $this->factory = new Factory;
    }
}
"#;

#[test]
fn a_member_assignment_records_a_member_initializer() {
    let (meta, symbols, refs) = php_flow(MEMBER_ASSIGNMENT);
    let property = symbols
        .iter()
        .position(|s| s.name == "factory" && s.kind == SymbolKind::Property)
        .expect("property symbol");
    assert_eq!(
        meta.flow_member_init.len(),
        1,
        "one member initializer: {meta:?}"
    );
    let (&ref_idx, &member_idx) = meta.flow_member_init.iter().next().unwrap();
    assert_eq!(member_idx, property);
    assert_eq!(refs[ref_idx].target_name, "Factory");
}

#[test]
fn a_member_assignment_adds_no_symbol() {
    let before = crate::languages::php::extract::extract(MEMBER_ASSIGNMENT)
        .symbols
        .len();
    let (_, symbols, _) = php_flow(MEMBER_ASSIGNMENT);
    assert_eq!(symbols.len(), before);
}

#[test]
fn a_plain_local_assignment_is_unaffected() {
    let source = "<?php\n\nfunction run()\n{\n    $client = make();\n    $client->go();\n}\n";
    let (meta, _, _) = php_flow(source);
    assert!(
        !meta.flow_binding_lhs.is_empty(),
        "a local assignment still seeds its binding: {meta:?}"
    );
    assert!(meta.flow_member_init.is_empty(), "{meta:?}");
}
