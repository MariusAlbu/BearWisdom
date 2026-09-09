use super::*;

#[test]
fn explicit_paths_are_decoded_once_and_unsupported_or_duplicate_paths_decline() {
    for (source, expected) in [
        (
            "#[path = \"other.rs\"] mod item;",
            Ok(Some("other.rs".into())),
        ),
        ("#[allow(dead_code)] mod item;", Ok(None)),
        ("#[path = \"a.rs\"] #[path = \"b.rs\"] mod item;", Err(())),
        ("#[path = concat!(\"a\", \".rs\")] mod item;", Err(())),
    ] {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let node = tree
            .root_node()
            .named_child(tree.root_node().named_child_count() - 1)
            .unwrap();
        assert_eq!(
            path_attribute(
                node,
                source.as_bytes(),
                crate::indexer::namespaces::syntax_for("rust").unwrap()
            ),
            expected
        );
    }
}
