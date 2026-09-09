use super::*;

#[test]
fn trait_headers_respect_ancestor_conditions_and_preserve_method_slots() {
    let source = "trait Save { fn save(&self); } struct Doc;
        #[cfg(feature=\"optional\")] impl Save for Doc { fn save(&self) {} }
        #[cfg(feature=\"optional\")] trait Hidden { fn hidden(&self); }";
    let extracted = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let data = capture(
        tree.root_node(),
        source.as_bytes(),
        syntax_for("rust").unwrap(),
        &extracted.symbols,
        &extracted.refs,
    );
    assert_eq!(data.traits.headers.len(), 3);
    assert!(data.traits.headers[0].enabled);
    assert!(data.traits.headers[1..].iter().all(|h| !h.enabled));
    let signature = data.traits.headers[0].members[0];
    assert_eq!(
        extracted.symbols[signature].kind,
        crate::types::SymbolKind::Method
    );
}
