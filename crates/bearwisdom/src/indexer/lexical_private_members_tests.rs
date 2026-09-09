use super::*;

#[test]
fn private_selector_rejects_missing_lexical_owner_and_static_or_duplicate_declarations() {
    let syntax = &crate::languages::typescript::flow::TS_LEXICAL_SYNTAX;
    for (source, bound) in [
        ("class Parent { #item: Doc; run() { this.#item.touch(); } }", true),
        ("class Parent { #item: Doc; } class Child extends Parent { run() { this.#item.touch(); } }", false),
        ("class Parent { static #item: Doc; run() { Parent.#item.touch(); } }", false),
        ("class Parent { #item: Doc; #item: Doc; run() { this.#item.touch(); } }", false),
    ] {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let byte = source.rfind("#item").unwrap();
        let token = tree.root_node().named_descendant_for_byte_range(byte, byte + 1).unwrap();
        assert_eq!(token.kind(), syntax.globals.private_member.0);
        let symbols = crate::languages::typescript::extract::extract(source, false).symbols;
        assert!(symbols.iter().any(|symbol| symbol.name == "#item"));
        let result = capture(token, source.as_bytes(), syntax, &symbols);
        assert_eq!(result.is_some(), bound, "{source}");
        if let Some(slot) = result { assert_eq!(symbols[slot].name, "#item"); }
    }
}
