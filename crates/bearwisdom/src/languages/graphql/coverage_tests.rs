// =============================================================================
// graphql/coverage_tests.rs
//
// Node-kind coverage for GraphQlPlugin::symbol_node_kinds() and ref_node_kinds().
// The grammar tree root for tree-sitter-graphql is:
//   source_file → document → definition → type_system_definition → object_type_definition
// The extractor descends through these wrapper nodes.
// =============================================================================

use crate::languages::LanguagePlugin;
use crate::languages::graphql::GraphQlPlugin;

// ---------------------------------------------------------------------------
// Grammar smoke tests — parse without errors
// ---------------------------------------------------------------------------

#[test]
fn cov_graphql_object_type_parses_cleanly() {
    let src = "type User {\n  name: String!\n  age: Int\n}";
    let lang: tree_sitter::Language = tree_sitter_graphql::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();
    let tree = parser.parse(src, None).unwrap();
    assert!(!tree.root_node().has_error(), "GraphQL type definition should parse without errors");
}

#[test]
fn cov_graphql_enum_parses_cleanly() {
    let src = "enum Role {\n  ADMIN\n  USER\n}";
    let lang: tree_sitter::Language = tree_sitter_graphql::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();
    let tree = parser.parse(src, None).unwrap();
    assert!(!tree.root_node().has_error(), "GraphQL enum should parse without errors");
}

// ---------------------------------------------------------------------------
// symbol_node_kinds and ref_node_kinds are declared
// ---------------------------------------------------------------------------

#[test]
fn cov_symbol_node_kinds_declared() {
    let plugin = GraphQlPlugin;
    assert!(
        plugin.symbol_node_kinds().contains(&"object_type_definition"),
        "object_type_definition in symbol_node_kinds"
    );
    assert!(
        plugin.symbol_node_kinds().contains(&"enum_type_definition"),
        "enum_type_definition in symbol_node_kinds"
    );
    assert!(
        plugin.ref_node_kinds().contains(&"named_type"),
        "named_type in ref_node_kinds"
    );
}

#[test]
fn cov_plugin_id_and_extensions() {
    let plugin = GraphQlPlugin;
    assert_eq!(plugin.id(), "graphql");
    assert!(plugin.extensions().contains(&".graphql") || plugin.extensions().contains(&".gql"));
}
