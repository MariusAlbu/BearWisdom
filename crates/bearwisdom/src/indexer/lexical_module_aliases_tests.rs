use super::*;

#[test]
fn assignments_and_alias_bases_have_separate_source_id_recipes() {
    let source = "declare module 'provider' { namespace API { function make(): void; } import Alias = API.make; export = Alias; } import API = require('provider');";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    assert!(!tree.root_node().has_error());
    let graph = crate::indexer::lexical::capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap();
    assert!(graph.module.complete);
    assert!(graph.module.units.iter().all(|u| u.complete));
    assert_eq!(graph.module.units[0].assignments.len(), 1);
    assert!(graph.module.units[0].exports.is_empty());
    assert!(graph
        .module
        .imports
        .values()
        .any(|i| matches!(i.source, ImportSource::Assignment(_))));
    assert!(graph
        .module
        .imports
        .values()
        .any(|i| matches!(i.source, ImportSource::Binding(Some(_))) && i.selectors == ["make"]));
    let namespace = &graph.module.units[1];
    let name = namespace.name.unwrap();
    let value = graph
        .lookup(graph.module.units[0].scope, name)
        .expect("namespace is bound in its enclosing module body");
    assert!(
        matches!(graph.module.imports[&value].source, ImportSource::Entity { namespace: id, .. } if id == namespace.id)
    );
}
