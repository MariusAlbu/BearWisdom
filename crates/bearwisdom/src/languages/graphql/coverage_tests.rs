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
use crate::types::{EdgeKind, SymbolKind};

fn extract(src: &str) -> crate::types::ExtractionResult {
    let plugin = GraphQlPlugin;
    plugin.extract(src, "", "graphql")
}


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

// ---------------------------------------------------------------------------
// object_type_definition → SymbolKind::Class
// ---------------------------------------------------------------------------

#[test]
fn cov_object_type_definition_emits_class() {
    let r = extract("type User {\n  id: ID!\n}");
    let sym = r.symbols.iter().find(|s| s.name == "User" && s.kind == SymbolKind::Class);
    assert!(sym.is_some(), "expected Class 'User'; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// interface_type_definition → SymbolKind::Interface
// ---------------------------------------------------------------------------

#[test]
fn cov_interface_type_definition_emits_interface() {
    let r = extract("interface Node {\n  id: ID!\n}");
    let sym = r.symbols.iter().find(|s| s.name == "Node" && s.kind == SymbolKind::Interface);
    assert!(sym.is_some(), "expected Interface 'Node'; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// enum_type_definition → SymbolKind::Enum
// ---------------------------------------------------------------------------

#[test]
fn cov_enum_type_definition_emits_enum() {
    let r = extract("enum Status {\n  ACTIVE\n  INACTIVE\n}");
    let sym = r.symbols.iter().find(|s| s.name == "Status" && s.kind == SymbolKind::Enum);
    assert!(sym.is_some(), "expected Enum 'Status'; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// enum_value_definition → SymbolKind::EnumMember (child of enum scope)
// ---------------------------------------------------------------------------

#[test]
fn cov_enum_value_definition_emits_enum_member() {
    let r = extract("enum Direction {\n  NORTH\n  SOUTH\n  EAST\n  WEST\n}");
    let members: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::EnumMember)
        .map(|s| s.name.as_str())
        .collect();
    for expected in &["NORTH", "SOUTH", "EAST", "WEST"] {
        assert!(members.contains(expected), "expected EnumMember '{expected}'; got: {members:?}");
    }
}

// ---------------------------------------------------------------------------
// union_type_definition → SymbolKind::Class
// ---------------------------------------------------------------------------

#[test]
fn cov_union_type_definition_emits_class() {
    let r = extract("union SearchResult = User | Post | Comment");
    let sym = r.symbols.iter().find(|s| s.name == "SearchResult" && s.kind == SymbolKind::Class);
    assert!(sym.is_some(), "expected Class 'SearchResult' for union; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// scalar_type_definition → SymbolKind::TypeAlias
// ---------------------------------------------------------------------------

#[test]
fn cov_scalar_type_definition_emits_type_alias() {
    let r = extract("scalar DateTime");
    let sym = r.symbols.iter().find(|s| s.name == "DateTime" && s.kind == SymbolKind::TypeAlias);
    assert!(sym.is_some(), "expected TypeAlias 'DateTime' for scalar; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// input_object_type_definition → SymbolKind::Struct
// ---------------------------------------------------------------------------

#[test]
fn cov_input_object_type_definition_emits_struct() {
    let r = extract("input CreateUserInput {\n  name: String!\n}");
    let sym = r.symbols.iter().find(|s| s.name == "CreateUserInput" && s.kind == SymbolKind::Struct);
    assert!(sym.is_some(), "expected Struct 'CreateUserInput' for input type; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// field_definition → SymbolKind::Field
// ---------------------------------------------------------------------------

#[test]
fn cov_field_definition_emits_field() {
    let r = extract("type Query {\n  user: User\n}");
    let field = r.symbols.iter().find(|s| s.name == "user" && s.kind == SymbolKind::Field);
    assert!(field.is_some(), "expected Field 'user'; got: {:?}", r.symbols);
}

/// field_definition TypeRef — `child_by_field_name("type")` returns None for this
/// grammar; the extractor now falls back to walking children by kind.
#[test]
fn cov_field_definition_type_ref_emitted() {
    let r = extract("type Query {\n  user: User\n}");
    let has_type_ref = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "User");
    assert!(has_type_ref, "expected TypeRef to 'User' from field return type; got: {:?}", r.refs);
}

/// non_null_type wrapper — TypeRef is unwrapped correctly.
#[test]
fn cov_field_definition_non_null_type_emits_type_ref() {
    let r = extract("type Query {\n  me: User!\n}");
    let field = r.symbols.iter().find(|s| s.name == "me" && s.kind == SymbolKind::Field);
    assert!(field.is_some(), "expected Field 'me'; got: {:?}", r.symbols);
    let has_type_ref = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "User");
    assert!(has_type_ref, "expected TypeRef 'User' from non_null field; got: {:?}", r.refs);
}

/// list_type wrapper — TypeRef is unwrapped correctly through [User!]!.
#[test]
fn cov_field_definition_list_type_emits_type_ref() {
    let r = extract("type Query {\n  users: [User!]!\n}");
    let field = r.symbols.iter().find(|s| s.name == "users" && s.kind == SymbolKind::Field);
    assert!(field.is_some(), "expected Field 'users'; got: {:?}", r.symbols);
    let has_type_ref = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "User");
    assert!(has_type_ref, "expected TypeRef 'User' from list field; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// directive_definition → SymbolKind::Function
// ---------------------------------------------------------------------------

#[test]
fn cov_directive_definition_emits_function() {
    let r = extract("directive @deprecated(reason: String) on FIELD_DEFINITION | ENUM_VALUE");
    let sym = r.symbols.iter().find(|s| s.name == "deprecated" && s.kind == SymbolKind::Function);
    assert!(sym.is_some(), "expected Function 'deprecated' for directive def; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// schema_definition → SymbolKind::Namespace + TypeRef to root operation types
// ---------------------------------------------------------------------------

#[test]
fn cov_schema_definition_emits_namespace_and_type_refs() {
    let r = extract("schema {\n  query: Query\n  mutation: Mutation\n}");
    let sym = r.symbols.iter().find(|s| s.name == "schema" && s.kind == SymbolKind::Namespace);
    assert!(sym.is_some(), "expected Namespace 'schema' for schema def; got: {:?}", r.symbols);
    let has_query_ref = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "Query");
    assert!(has_query_ref, "expected TypeRef to 'Query' from schema def; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// operation_definition → SymbolKind::Function
// ---------------------------------------------------------------------------

#[test]
fn cov_operation_definition_query_emits_function() {
    let r = extract("query GetUser {\n  user {\n    id\n  }\n}");
    let sym = r.symbols.iter().find(|s| s.name == "GetUser" && s.kind == SymbolKind::Function);
    assert!(sym.is_some(), "expected Function 'GetUser' from operation def; got: {:?}", r.symbols);
}

#[test]
fn cov_operation_definition_mutation_emits_function() {
    let r = extract("mutation CreatePost($title: String!) {\n  createPost(title: $title) {\n    id\n  }\n}");
    let sym = r.symbols.iter().find(|s| s.name == "CreatePost" && s.kind == SymbolKind::Function);
    assert!(sym.is_some(), "expected Function 'CreatePost' from mutation def; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// fragment_definition → SymbolKind::Function + TypeRef to on-type
// ---------------------------------------------------------------------------

#[test]
fn cov_fragment_definition_emits_function_and_type_ref() {
    let r = extract("fragment UserFields on User {\n  id\n  name\n}");
    let sym = r.symbols.iter().find(|s| s.name == "UserFields" && s.kind == SymbolKind::Function);
    assert!(sym.is_some(), "expected Function 'UserFields' from fragment def; got: {:?}", r.symbols);
    let has_type_ref = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "User");
    assert!(has_type_ref, "expected TypeRef to 'User' from fragment on-type; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// input_value_definition → SymbolKind::Field + TypeRef
// ---------------------------------------------------------------------------

#[test]
fn cov_input_value_definition_emits_field_and_type_ref() {
    let r = extract("input CreatePostInput {\n  authorId: ID!\n  category: Category\n}");
    let field = r.symbols.iter().find(|s| s.name == "category" && s.kind == SymbolKind::Field);
    assert!(field.is_some(), "expected Field 'category' inside input type; got: {:?}", r.symbols);
    let has_type_ref = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "Category");
    assert!(has_type_ref, "expected TypeRef to 'Category' from input_value_definition; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// implements_interfaces → EdgeKind::Implements
// ---------------------------------------------------------------------------

/// Single interface — direct named_type child, fully handled.
#[test]
fn cov_implements_single_interface_emits_implements_edge() {
    let r = extract("type Dog implements Animal {\n  name: String\n}");
    let has_animal = r.refs.iter().any(|rf| rf.kind == EdgeKind::Implements && rf.target_name == "Animal");
    assert!(has_animal, "expected Implements edge to 'Animal'; got: {:?}", r.refs);
}

/// Multiple interfaces with & — implements_interfaces is left-recursive; extractor
/// now recursively collects all interfaces.
#[test]
fn cov_implements_multiple_interfaces_both_emitted() {
    let r = extract("type Dog implements Animal & Pet {\n  name: String\n}");
    let has_animal = r.refs.iter().any(|rf| rf.kind == EdgeKind::Implements && rf.target_name == "Animal");
    assert!(has_animal, "expected Implements edge to 'Animal'; got: {:?}", r.refs);
    let has_pet = r.refs.iter().any(|rf| rf.kind == EdgeKind::Implements && rf.target_name == "Pet");
    assert!(has_pet, "expected Implements edge to 'Pet'; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// Type extensions — wrapped in type_system_extension → type_extension
// ---------------------------------------------------------------------------

/// object_type_extension emits TypeRef to the extended type.
#[test]
fn cov_object_type_extension_emits_type_ref() {
    let r = extract("extend type User {\n  email: String\n}");
    let has_ref = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "User");
    assert!(has_ref, "expected TypeRef to 'User' from type extension; got: {:?}", r.refs);
}

/// union_type_definition TypeRef — all members are collected via recursive union_member_types walk.
#[test]
fn cov_union_type_definition_all_members_emitted() {
    let r = extract("union SearchResult = User | Post | Comment");
    let sym = r.symbols.iter().find(|s| s.name == "SearchResult" && s.kind == SymbolKind::Class);
    assert!(sym.is_some(), "expected Class 'SearchResult'; got: {:?}", r.symbols);
    for expected in &["User", "Post", "Comment"] {
        let has = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == *expected);
        assert!(has, "expected TypeRef to union member '{}'; got: {:?}", expected, r.refs);
    }
}
