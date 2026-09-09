use super::*;

#[test]
fn rowless_call_and_construct_type_parameters_have_distinct_scopes() {
    let source = "interface Factory { <T>(value: T): T; new<T>(value: T): T; }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let graph = crate::indexer::lexical::capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap();
    let name = graph.name_id("T").unwrap();
    let calls: Vec<_> = source
        .match_indices("value: T")
        .map(|(byte, _)| graph.type_binding_at((byte + 7) as u32, name).unwrap())
        .collect();
    assert_ne!(
        calls[0], calls[1],
        "sibling signatures must not share a generic binding"
    );
    assert_eq!(
        graph.type_binding_at(source.find('}').unwrap() as u32, name),
        None,
        "signature generic must not leak into its containing interface"
    );
}

#[test]
fn type_namespace_ignores_value_only_shadowing() {
    let mut graph = LexicalBindings::default();
    let root = graph.add_scope(None, 0, 100, true);
    let inner = graph.add_scope(Some(root), 20, 80, true);
    let name = graph.intern("Model");
    let outer = graph.declare_type(root, name);
    let value = graph.declare(inner, name, 20, None);
    assert_ne!(value, outer);
    assert_eq!(graph.type_binding_at(40, name), Some(outer));
    assert_eq!(graph.binding_at(40, name), Some(value));
    let local_type = graph.declare_type(inner, name);
    assert_eq!(graph.type_binding_at(40, name), Some(local_type));
    assert_eq!(graph.type_binding_at(90, name), Some(outer));
}

#[test]
fn value_expression_domain_skips_pure_types_but_stops_at_imports_in_the_nearer_scope() {
    let mut graph = LexicalBindings::default();
    let root = graph.add_scope(None, 0, 100, true);
    let inner = graph.add_scope(Some(root), 20, 80, true);
    let name = graph.intern("Token");
    let ty = graph.declare_type(inner, name);
    assert_eq!(
        graph.value_expression_binding_at(40, name),
        None,
        "configured value lookup may follow, never the type ID"
    );
    let value = graph.declare(root, name, 0, None);
    assert_eq!(graph.value_expression_binding_at(40, name), Some(value));
    assert_eq!(graph.type_binding_at(40, name), Some(ty));
    graph.module.imports.insert(
        ty,
        super::super::modules::Import {
            source: super::super::modules::ImportSource::Named {
                module: "provider".into(),
                name: "Token".into(),
            },
            type_only: true,
            selectors: vec![],
        },
    );
    assert_eq!(
        graph.value_expression_binding_at(40, name),
        Some(ty),
        "type-only import blocks an outer value namesake"
    );
    assert_eq!(graph.value_expression_binding_at(90, name), Some(value));
    let local = graph.declare(inner, name, 20, None);
    assert_eq!(graph.value_expression_binding_at(40, name), Some(local));
}

#[test]
fn value_expression_lookup_keeps_ordered_declarations_and_capture_barriers() {
    let mut graph = LexicalBindings::default();
    let root = graph.add_scope(None, 0, 100, true);
    let inner = graph.add_scope(Some(root), 20, 80, true);
    let name = graph.intern("item");
    let outer = graph.declare(root, name, 0, None);
    let first = graph.declare_ordered(inner, name, 30, None);
    let second = graph.declare_ordered(inner, name, 50, None);
    assert_eq!(graph.value_expression_binding_at(25, name), Some(outer));
    assert_eq!(graph.value_expression_binding_at(40, name), Some(first));
    assert_eq!(graph.value_expression_binding_at(60, name), Some(second));
    graph.capture_barriers.insert(inner);
    assert_eq!(graph.value_expression_binding_at(25, name), None);
    assert_eq!(graph.value_expression_binding_at(60, name), Some(second));
}
