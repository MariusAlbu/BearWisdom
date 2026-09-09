use super::*;

#[test]
fn ambient_exports_keep_named_declarations_and_annotations() {
    let source = "export interface Result { refetch(): void; } export declare function create(): Result; export declare const api: Result;";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let graph = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap();
    assert_eq!(
        graph.module.exports.len(),
        3,
        "{}",
        tree.root_node().to_sexp()
    );
    for name in ["create", "api"] {
        let export = graph
            .module
            .exports
            .iter()
            .find(|export| export.name == name)
            .unwrap();
        assert!(
            matches!(export.target, ExportTarget::Local { value: Some(_), .. }),
            "{export:?}"
        );
    }
}

#[test]
fn namespace_export_is_an_explicit_module_object_not_a_wildcard() {
    let source = "export * as NS from './model'; export * from './other';";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let graph = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap();
    assert_eq!(graph.module.stars, [("./other".into(), false)]);
    assert_eq!(graph.module.exports.len(), 1);
    assert_eq!(graph.module.exports[0].name, "NS");
    assert!(
        matches!(&graph.module.exports[0].target, ExportTarget::From(import) if matches!(&import.source, ImportSource::Namespace(module) if module == "./model"))
    );
}

#[test]
fn imports_bind_aliases_before_export_aliases_and_preserve_type_space() {
    let source = "import Default, { Model as Alias, type Shape } from './model'; export { Alias as Public }; export { Model as Other } from './other'; export * from './star';";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let graph = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap();
    assert_eq!(graph.module.imports.len(), 3);
    let alias = graph
        .binding_at(0, graph.name_id("Alias").unwrap())
        .unwrap();
    assert!(
        matches!(&graph.module.imports[&alias].source, ImportSource::Named { module, name } if name == "Model" && module == "./model")
    );
    let shape = graph
        .type_binding_at(0, graph.name_id("Shape").unwrap())
        .unwrap();
    assert!(graph.module.imports[&shape].type_only);
    assert_eq!(graph.binding_at(0, graph.name_id("Shape").unwrap()), None);
    assert!(
        matches!(graph.module.exports[0].target, ExportTarget::Local { value: Some(id), .. } if id == alias)
    );
    assert_eq!(graph.module.exports[0].name, "Public");
    assert!(
        matches!(&graph.module.exports[1].target, ExportTarget::From(import) if matches!(&import.source, ImportSource::Named {module, name} if name == "Model" && module == "./other"))
    );
    assert_eq!(graph.module.stars, [("./star".to_owned(), false)]);
}
