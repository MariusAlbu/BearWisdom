use super::*;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Parser;

/// Parse `source` and return its tree.
fn parse(source: &str) -> tree_sitter::Tree {
    let language = tree_sitter_go::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("failed to set Go grammar");
    parser.parse(source, None).expect("failed to parse")
}

fn find_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_kind(child, kind) {
            return Some(found);
        }
    }
    None
}

/// The first `parameter_list` node in `tree` — a `method_declaration`'s
/// receiver list, since it's the first one encountered depth-first.
fn first_parameter_list(tree: &tree_sitter::Tree) -> Node<'_> {
    find_kind(tree.root_node(), "parameter_list").expect("expected a parameter_list")
}

#[test]
fn receiver_type_from_value_receiver() {
    let src = "package p\ntype Point struct{}\nfunc (p Point) M() {}";
    let tree = parse(src);
    let params = first_parameter_list(&tree);
    assert_eq!(
        extract_receiver_type_from_param_list(&params, src),
        Some("Point".to_string())
    );
}

#[test]
fn receiver_type_from_pointer_receiver() {
    let src = "package p\ntype Server struct{}\nfunc (s *Server) M() {}";
    let tree = parse(src);
    let params = first_parameter_list(&tree);
    assert_eq!(
        extract_receiver_type_from_param_list(&params, src),
        Some("Server".to_string())
    );
}

#[test]
fn typed_param_emits_property_symbol_and_type_ref() {
    let src = "package p\nfunc GetUser(repo UserRepository, id int) {}";
    let tree = parse(src);
    let params = first_parameter_list(&tree);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();
    extract_go_typed_params_as_symbols(&params, src, &mut symbols, &mut refs, None, "p.GetUser");

    let repo = symbols
        .iter()
        .find(|s| s.name == "repo")
        .expect("expected Property symbol 'repo'");
    assert_eq!(repo.kind, SymbolKind::Property);
    assert_eq!(repo.qualified_name, "p.GetUser.repo");

    // `id int` is builtin-typed — no symbol, no ref.
    assert!(symbols.iter().all(|s| s.name != "id"));

    let type_ref = refs
        .iter()
        .find(|r| r.kind == EdgeKind::TypeRef)
        .expect("expected a TypeRef for 'repo'");
    assert_eq!(type_ref.target_name, "UserRepository");
    assert_eq!(type_ref.module, None);
}

#[test]
fn qualified_typed_param_carries_module_and_faithful_signature() {
    let src = "package p\nfunc TestSomething(t *testing.T) {}";
    let tree = parse(src);
    let params = first_parameter_list(&tree);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();
    extract_go_typed_params_as_symbols(
        &params,
        src,
        &mut symbols,
        &mut refs,
        None,
        "p.TestSomething",
    );

    let param = symbols
        .iter()
        .find(|s| s.name == "t")
        .expect("expected Property symbol 't'");
    assert_eq!(param.signature.as_deref(), Some("t *testing.T"));

    let type_ref = refs
        .iter()
        .find(|r| r.kind == EdgeKind::TypeRef)
        .expect("expected a TypeRef for 't'");
    assert_eq!(type_ref.target_name, "T");
    assert_eq!(type_ref.module.as_deref(), Some("testing"));
}
