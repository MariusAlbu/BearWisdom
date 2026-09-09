use super::*;

fn ambient_graph(source: &str) -> LexicalBindings {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap()
}

#[test]
fn ambient_module_vars_have_distinct_owners_and_cannot_escape_to_the_file() {
    let source = "declare module 'a' { var item: A; type First = typeof item; } declare module 'b' { var item: B; type Second = typeof item; } item;";
    let graph = ambient_graph(source);
    let name = graph.name_id("item").unwrap();
    let bindings: Vec<_> = source
        .match_indices("typeof item")
        .map(|(byte, _)| graph.binding_at(byte as u32, name).unwrap())
        .collect();
    assert_ne!(bindings[0], bindings[1]);
    assert_ne!(
        graph.bindings[bindings[0].0].scope,
        graph.bindings[bindings[1].0].scope
    );
    assert_eq!(
        graph.binding_at(source.rfind("item;").unwrap() as u32, name),
        None
    );
}

#[test]
fn ambient_module_imports_bind_in_their_own_scope_before_type_uses() {
    let source = "declare module 'a' { import { Doc as Item } from 'left'; type First = Item; } declare module 'b' { import { Doc as Item } from 'right'; type Second = Item; }";
    let graph = ambient_graph(source);
    let name = graph
        .name_id("Item")
        .expect("nested import identifiers must be captured");
    let bindings: Vec<_> = source
        .match_indices("= Item")
        .map(|(byte, _)| graph.type_binding_at(byte as u32 + 2, name).unwrap())
        .collect();
    assert_ne!(bindings[0], bindings[1]);
    assert!(
        matches!(&graph.module.imports[&bindings[0]].source, super::super::modules::ImportSource::Named { module, .. } if module == "left")
    );
    assert!(
        matches!(&graph.module.imports[&bindings[1]].source, super::super::modules::ImportSource::Named { module, .. } if module == "right")
    );
    assert_eq!(graph.binding_at(0, name), None);
    assert_eq!(graph.type_binding_at(0, name), None);
}

#[test]
fn syntax_selection_lets_contract_restoration_skip_unmigrated_grammars() {
    assert!(syntax_for("ts").is_some());
    assert!(std::ptr::eq(
        syntax_for("ts").unwrap(),
        syntax_for("js").unwrap()
    ));
    for prefix in ["rs", "py", "cs", "java", ""] {
        assert!(syntax_for(prefix).is_none());
    }
}

#[test]
fn reference_roots_decode_source_tokens_and_type_only_values_are_fenced() {
    let source = "import { Model as Item } from './model'; import type { Shape } from './shape'; function f(Item) { Item.save(); new Shape(); }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let graph = capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap();
    let syntax = &crate::languages::typescript::flow::TS_LEXICAL_SYNTAX;
    for (expression, spelling) in [("Item.save()", "Item"), ("new Shape()", "Shape")] {
        let byte = source.find(expression).unwrap() as u32;
        let token = reference_root(tree.root_node(), byte, syntax).unwrap();
        assert_eq!(token.utf8_text(source.as_bytes()).unwrap(), spelling);
        let name = graph.name_id(spelling).unwrap();
        let binding = graph.reference_binding_at(byte, name).unwrap();
        if spelling == "Item" {
            assert_eq!(graph.kinds[&binding], SymbolKind::Parameter);
            assert!(!graph.module.imports.contains_key(&binding));
        } else {
            assert!(graph.module.imports[&binding].type_only);
            assert_eq!(graph.binding_at(byte, name), None);
        }
    }
}

#[test]
fn expression_self_environment_is_private_and_outside_parameter_and_var_scopes() {
    let source = "const run = function self(self) { self(); }; const other = function self() { var self; self(); }; self();";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let graph = capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap();
    let name = graph.name_id("self").unwrap();
    let uses: Vec<_> = source
        .match_indices("self();")
        .map(|(byte, _)| graph.binding_at(byte as u32, name))
        .collect();
    let parameter = uses[0].unwrap();
    let variable = uses[1].unwrap();
    assert_eq!(uses[2], None);
    assert_ne!(parameter, variable);
    assert_eq!(graph.kinds[&parameter], SymbolKind::Parameter);
    assert_eq!(graph.kinds[&variable], SymbolKind::Variable);
    assert_eq!(graph.lexical_only.len(), 2);
    assert_eq!(graph.initial_values.len(), 2);
    for (&outer, &private) in &graph.initial_values {
        assert_ne!(outer, private);
        assert!(graph.lexical_only.contains(&private));
        assert!(!graph.lexical_only.contains(&outer));
        assert!(![parameter, variable].contains(&private));
        let self_scope = graph.bindings[private.0].scope;
        assert!([parameter, variable].iter().any(|&binding| graph.scopes
            [graph.bindings[binding.0].scope.0]
            .parent
            == Some(self_scope)));
    }
}

#[test]
fn root_type_arguments_use_expression_anchors_without_capturing_member_arguments() {
    let source = "function f() { make<A[]>().next<B>(); new Channel<C>().each<D>(); }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let graph = capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap();
    assert_eq!(graph.call_type_args.len(), 2);
    assert_eq!(
        graph.call_type_args[&(source.find("make<").unwrap() as u32)],
        vec!["A[]"]
    );
    assert_eq!(
        graph.call_type_args[&(source.find("new Channel").unwrap() as u32)],
        vec!["C"]
    );
    assert_eq!(graph.types.member_arguments.len(), 2);
    for (selector, argument) in [("next<", "B"), ("each<", "D")] {
        let recipes = &graph.types.member_arguments[&(source.find(selector).unwrap() as u32)];
        assert!(
            matches!(&recipes[..], [super::super::type_syntax::TypeExpr::Global { name, legacy }]
            if Some(*name) == graph.name_id(argument) && legacy == argument)
        );
    }
}

#[test]
fn destructuring_binds_values_not_property_keys_and_unknown_profiles_stay_legacy() {
    let source = "function f({ key: local, shorthand, ...rest }) { local; shorthand; rest; }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let root = tree.root_node();
    let mut symbols = Vec::new();
    let policy = crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly;
    assert!(capture(
        root,
        source.as_bytes(),
        "unmigrated",
        &mut symbols,
        &[],
        policy
    )
    .is_none());
    let graph = capture(root, source.as_bytes(), "ts", &mut symbols, &[], policy).unwrap();
    assert_eq!(graph.name_id("key"), None);
    let cursor = source.find("local;").unwrap() as u32;
    for name in ["local", "shorthand", "rest"] {
        assert!(graph
            .binding_at(cursor, graph.name_id(name).unwrap())
            .is_some());
    }
}
