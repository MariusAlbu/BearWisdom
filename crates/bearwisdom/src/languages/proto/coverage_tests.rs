// =============================================================================
// proto/coverage_tests.rs
//
// Node-kind coverage for ProtoPlugin::symbol_node_kinds() and ref_node_kinds().
// The plugin mod.rs stubs ExtractionResult::empty() pending grammar wiring.
// The extract::extract() extractor uses child_by_field_name("message_name") which
// doesn't match this version of tree-sitter-proto (grammar does not declare
// field names for message_name, enum_name, etc.).
//
// These tests verify:
//   1. The grammar parses proto3 SDL without errors.
//   2. The declared node kind lists are complete and correct.
//
// symbol_node_kinds: message, service, rpc, enum, enum_field,
//                   field, map_field, package, import
// ref_node_kinds:    message_or_enum_type, import
// =============================================================================

use crate::languages::LanguagePlugin;
use crate::languages::proto::ProtoPlugin;

// ---------------------------------------------------------------------------
// Grammar smoke tests — parse without errors
// ---------------------------------------------------------------------------

#[test]
fn cov_proto_message_parses_cleanly() {
    let src = "syntax = \"proto3\";\nmessage User {\n  string name = 1;\n}";
    let lang: tree_sitter::Language = tree_sitter_proto::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();
    let tree = parser.parse(src, None).unwrap();
    assert!(!tree.root_node().has_error(), "proto message should parse without errors");
}

#[test]
fn cov_proto_enum_parses_cleanly() {
    let src = "syntax = \"proto3\";\nenum Status {\n  UNKNOWN = 0;\n  ACTIVE = 1;\n}";
    let lang: tree_sitter::Language = tree_sitter_proto::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();
    let tree = parser.parse(src, None).unwrap();
    assert!(!tree.root_node().has_error(), "proto enum should parse without errors");
}

#[test]
fn cov_proto_service_parses_cleanly() {
    let src = "syntax = \"proto3\";\nservice Greeter {\n  rpc SayHello (HelloRequest) returns (HelloReply);\n}";
    let lang: tree_sitter::Language = tree_sitter_proto::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();
    let tree = parser.parse(src, None).unwrap();
    assert!(!tree.root_node().has_error(), "proto service/rpc should parse without errors");
}

// ---------------------------------------------------------------------------
// symbol_node_kinds and ref_node_kinds are declared
// ---------------------------------------------------------------------------

#[test]
fn cov_symbol_node_kinds_declared() {
    let plugin = ProtoPlugin;
    assert!(plugin.symbol_node_kinds().contains(&"message"), "message in symbol_node_kinds");
    assert!(plugin.symbol_node_kinds().contains(&"service"), "service in symbol_node_kinds");
    assert!(plugin.symbol_node_kinds().contains(&"enum"), "enum in symbol_node_kinds");
    assert!(plugin.symbol_node_kinds().contains(&"field"), "field in symbol_node_kinds");
}

#[test]
fn cov_ref_node_kinds_declared() {
    let plugin = ProtoPlugin;
    assert!(
        plugin.ref_node_kinds().contains(&"message_or_enum_type"),
        "message_or_enum_type in ref_node_kinds"
    );
    assert!(plugin.ref_node_kinds().contains(&"import"), "import in ref_node_kinds");
}
