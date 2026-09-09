use super::*;

fn graph(source: &str) -> LexicalBindings {
    let grammar: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).unwrap();
    let tree = parser.parse(source, None).unwrap();
    capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut Vec::new(),
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap()
}

#[test]
fn sibling_parameters_are_distinct_bindings_even_on_one_line() {
    let src = "function a(x: A) { x.save(); } function b(x: B) { x.save(); }";
    let model = graph(src);
    let name = model.name_id("x").unwrap();
    let first = model
        .binding_at(src.find("x.save").unwrap() as u32, name)
        .unwrap();
    let second = model
        .binding_at(src.rfind("x.save").unwrap() as u32, name)
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(model.bindings[first.0].annotation.as_deref(), Some("A"));
    assert_eq!(model.bindings[second.0].annotation.as_deref(), Some("B"));
}

#[test]
fn blocks_closures_and_tdz_select_the_declaration_not_latest_spelling() {
    let src = "function f(x: A) { { x.before(); let x: B; x.inside(); } x.after(); const cb = (x: C) => x.lambda(); }";
    let model = graph(src);
    let name = model.name_id("x").unwrap();
    let at = |text: &str| {
        model
            .binding_at(src.find(text).unwrap() as u32, name)
            .unwrap()
    };
    assert_eq!(
        at("x.before"),
        at("x.inside"),
        "TDZ must shadow, not select the outer parameter"
    );
    assert_ne!(at("x.inside"), at("x.after"));
    assert_ne!(at("x.lambda"), at("x.after"));
}

#[test]
fn var_uses_function_scope_but_let_uses_block_scope() {
    let src = "function f() { { var a = 1; let b = 1; } a; b; }";
    let model = graph(src);
    let cursor = src.rfind("a; b;").unwrap() as u32;
    assert!(model
        .binding_at(cursor, model.name_id("a").unwrap())
        .is_some());
    assert!(model
        .binding_at(cursor, model.name_id("b").unwrap())
        .is_none());
}

#[test]
fn hoisted_declaration_shadows_parameter_only_inside_its_block() {
    let src = "function f(make: () => A) { { make(); function make(): B {} } make(); }";
    let model = graph(src);
    let name = model.name_id("make").unwrap();
    let inner = model
        .binding_at(src.find("make();").unwrap() as u32, name)
        .unwrap();
    let outer = model
        .binding_at(src.rfind("make();").unwrap() as u32, name)
        .unwrap();
    assert_ne!(inner, outer);
    assert_eq!(model.kinds[&inner], crate::types::SymbolKind::Function);
    assert_eq!(model.kinds[&outer], crate::types::SymbolKind::Parameter);
}
#[test]
fn ordered_shadowing_keeps_initializer_reads_on_the_previous_binding() {
    let mut graph = super::LexicalBindings::default();
    let scope = graph.add_scope(None, 0, 100, true);
    let name = graph.intern("item");
    let first = graph.declare_ordered(scope, name, 10, None);
    let second = graph.declare_ordered(scope, name, 40, None);
    assert_ne!(first, second);
    assert_eq!(graph.binding_at(5, name), None);
    assert_eq!(graph.binding_at(35, name), Some(first));
    assert_eq!(graph.binding_at(40, name), Some(second));
    let child = graph.add_scope(Some(scope), 50, 90, true);
    graph.capture_barriers.insert(child);
    assert_eq!(graph.binding_at(60, name), None);
}
