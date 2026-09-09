use super::*;

#[test]
fn blanket_arguments_correspond_by_binding_id_and_specialization_is_not_a_blanket() {
    for (head, expected) in [
        ("impl<T,U> Pair<U,T>", 2),
        ("impl<T> Pair<T,T>", 0),
        ("impl<T: Clone,U> Pair<T,U>", 0),
        ("impl Pair<u32,u32>", 0),
    ] {
        let source = format!("struct Pair<A,B> {{ a: A, b: B }} {head} {{ fn get(&self) {{}} }}");
        let extracted = crate::languages::rust_lang::extract::extract(&source);
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(&source, None).unwrap();
        let data = capture(
            tree.root_node(),
            source.as_bytes(),
            syntax_for("rust").unwrap(),
            &extracted.symbols,
            &extracted.refs,
        );
        let extension = &data.extensions[0];
        assert_eq!(data.extension_parameters.len(), expected, "{source}");
        assert_eq!(
            matches!(data.bindings[extension.owner.0].targets[0], Target::Missing),
            expected == 0
        );
    }
}
