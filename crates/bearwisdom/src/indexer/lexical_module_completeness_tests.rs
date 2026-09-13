use super::surface_complete;

fn parse(source: &str) -> tree_sitter::Tree {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    parser.parse(source, None).unwrap()
}

fn forms() -> &'static super::ModuleForms {
    crate::languages::typescript::flow::TS_LEXICAL_SYNTAX.modules
}

/// A grammar gap inside a type alias's object type (a mapped-type modifier
/// written `]? :`) does not touch the names the module exports.
#[test]
fn an_error_inside_a_declaration_body_leaves_the_surface_complete() {
    let src = "import { Base } from './base';\n\
        type Partialish<T> = { [K in keyof T]? : T[K] };\n\
        interface Check<T> { toBe(value: T): void }\n\
        export { Check, Partialish };\n";
    let tree = parse(src);
    assert!(
        tree.root_node().has_error(),
        "the fixture must exercise a real grammar gap"
    );
    assert!(surface_complete(tree.root_node(), forms()));
}

/// An error in an export clause may have swallowed a name: incomplete.
#[test]
fn an_error_in_an_export_clause_leaves_the_surface_incomplete() {
    let src = "interface Check<T> { toBe(value: T): void }\nexport { Check, % };\n";
    let tree = parse(src);
    assert!(tree.root_node().has_error());
    assert!(!surface_complete(tree.root_node(), forms()));
}

/// An error-free tree is complete without any walk.
#[test]
fn an_error_free_tree_is_complete() {
    let tree = parse("export const x = 1;\n");
    assert!(surface_complete(tree.root_node(), forms()));
}

/// A stray token between top-level statements is not inside any declaration.
#[test]
fn an_error_between_statements_leaves_the_surface_incomplete() {
    let src = "export const x = 1;\n%\nexport const y = 2;\n";
    let tree = parse(src);
    assert!(tree.root_node().has_error());
    assert!(!surface_complete(tree.root_node(), forms()));
}
