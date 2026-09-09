use super::*;

#[test]
fn empty_anchor_sets_cannot_invent_an_owner_row() {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let source = "struct Model; impl Model { fn save() {} }";
    let tree = parser.parse(source, None).unwrap();
    let forms = Forms {
        scopes: &["impl_item"],
        declarations: &[("struct_item", true)],
        isolated_scopes: &[],
        opaque_bindings: &[],
        excluded_fields: &[],
        imports: None,
        name_prefixes: &[],
        extension: "impl_item",
        target: "type",
        wrappers: &[],
        identifiers: &["type_identifier"],
        members: &["function_item"],
    };
    capture(tree.root_node(), source.as_bytes(), &forms, &mut []);
}
