// =============================================================================
// proto/coverage_tests.rs
//
// Node-kind coverage for ProtoPlugin::symbol_node_kinds() and ref_node_kinds().
//
// symbol_node_kinds: message, service, rpc, enum, enum_field,
//                   field, map_field, package, import
// ref_node_kinds:    message_or_enum_type, import
// =============================================================================

use crate::languages::LanguagePlugin;
use crate::languages::proto::ProtoPlugin;
use crate::types::{EdgeKind, SymbolKind};

fn extract(src: &str) -> crate::types::ExtractionResult {
    let plugin = ProtoPlugin;
    plugin.extract(src, "", "proto")
}

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

// ---------------------------------------------------------------------------
// message → SymbolKind::Struct
// ---------------------------------------------------------------------------

#[test]
fn cov_message_emits_struct() {
    let r = extract("syntax = \"proto3\";\nmessage UserProfile {\n  string name = 1;\n}");
    let sym = r.symbols.iter().find(|s| s.name == "UserProfile" && s.kind == SymbolKind::Struct);
    assert!(sym.is_some(), "expected Struct 'UserProfile'; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// service → SymbolKind::Interface
// ---------------------------------------------------------------------------

#[test]
fn cov_service_emits_interface() {
    let r = extract("syntax = \"proto3\";\nservice UserService {\n  rpc GetUser (GetUserRequest) returns (GetUserResponse);\n}");
    let sym = r.symbols.iter().find(|s| s.name == "UserService" && s.kind == SymbolKind::Interface);
    assert!(sym.is_some(), "expected Interface 'UserService'; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// rpc → SymbolKind::Method + TypeRef for request and response types
// ---------------------------------------------------------------------------

#[test]
fn cov_rpc_emits_method() {
    let r = extract("syntax = \"proto3\";\nservice Greeter {\n  rpc SayHello (HelloRequest) returns (HelloReply);\n}");
    let sym = r.symbols.iter().find(|s| s.name == "SayHello" && s.kind == SymbolKind::Method);
    assert!(sym.is_some(), "expected Method 'SayHello'; got: {:?}", r.symbols);
}

#[test]
fn cov_rpc_emits_type_refs_for_request_and_response() {
    let r = extract("syntax = \"proto3\";\nservice Greeter {\n  rpc SayHello (HelloRequest) returns (HelloReply);\n}");
    let has_req = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "HelloRequest");
    let has_resp = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "HelloReply");
    assert!(has_req, "expected TypeRef to 'HelloRequest'; got: {:?}", r.refs);
    assert!(has_resp, "expected TypeRef to 'HelloReply'; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// enum → SymbolKind::Enum
// ---------------------------------------------------------------------------

#[test]
fn cov_enum_emits_enum() {
    let r = extract("syntax = \"proto3\";\nenum OrderStatus {\n  PENDING = 0;\n  SHIPPED = 1;\n}");
    let sym = r.symbols.iter().find(|s| s.name == "OrderStatus" && s.kind == SymbolKind::Enum);
    assert!(sym.is_some(), "expected Enum 'OrderStatus'; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// enum_field → SymbolKind::EnumMember (child of enum scope)
// ---------------------------------------------------------------------------

#[test]
fn cov_enum_field_emits_enum_member() {
    let r = extract("syntax = \"proto3\";\nenum Color {\n  RED = 0;\n  GREEN = 1;\n  BLUE = 2;\n}");
    let members: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::EnumMember)
        .map(|s| s.name.as_str())
        .collect();
    for expected in &["RED", "GREEN", "BLUE"] {
        assert!(members.contains(expected), "expected EnumMember '{expected}'; got: {members:?}");
    }
}

// ---------------------------------------------------------------------------
// field → SymbolKind::Field + TypeRef for non-primitive type
// ---------------------------------------------------------------------------

#[test]
fn cov_field_primitive_emits_field_no_type_ref() {
    let r = extract("syntax = \"proto3\";\nmessage Msg {\n  string title = 1;\n  int32 count = 2;\n}");
    let title = r.symbols.iter().find(|s| s.name == "title" && s.kind == SymbolKind::Field);
    assert!(title.is_some(), "expected Field 'title'; got: {:?}", r.symbols);
    // Primitive string should not produce a TypeRef
    let primitive_ref = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "string");
    assert!(!primitive_ref, "should not emit TypeRef for primitive 'string'; got: {:?}", r.refs);
}

#[test]
fn cov_field_message_type_emits_type_ref() {
    let r = extract("syntax = \"proto3\";\nmessage Order {\n  Address shipping_address = 1;\n}");
    let field = r.symbols.iter().find(|s| s.name == "shipping_address" && s.kind == SymbolKind::Field);
    assert!(field.is_some(), "expected Field 'shipping_address'; got: {:?}", r.symbols);
    let has_ref = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "Address");
    assert!(has_ref, "expected TypeRef to 'Address' from message field; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// map_field → SymbolKind::Field
// ---------------------------------------------------------------------------

#[test]
fn cov_map_field_emits_field() {
    let r = extract("syntax = \"proto3\";\nmessage Config {\n  map<string, string> labels = 1;\n}");
    let sym = r.symbols.iter().find(|s| s.name == "labels" && s.kind == SymbolKind::Field);
    assert!(sym.is_some(), "expected Field 'labels' from map_field; got: {:?}", r.symbols);
}

/// map_field with message value type → TypeRef to value type
#[test]
fn cov_map_field_message_value_emits_type_ref() {
    let r = extract("syntax = \"proto3\";\nmessage Registry {\n  map<string, Service> services = 1;\n}");
    let has_ref = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "Service");
    assert!(has_ref, "expected TypeRef to 'Service' from map_field value type; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// package → SymbolKind::Namespace
// ---------------------------------------------------------------------------

#[test]
fn cov_package_emits_namespace() {
    let r = extract("syntax = \"proto3\";\npackage com.example.api;\n");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Namespace && s.name.contains("com"));
    assert!(sym.is_some(), "expected Namespace for package declaration; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// import → Imports edge
// ---------------------------------------------------------------------------

#[test]
fn cov_import_emits_imports_edge() {
    let r = extract("syntax = \"proto3\";\nimport \"google/protobuf/timestamp.proto\";\n");
    let has_import = r.refs.iter().any(|rf| {
        rf.kind == EdgeKind::Imports && rf.target_name.contains("timestamp")
    });
    assert!(has_import, "expected Imports edge for import statement; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// oneof → scope with Field children
// ---------------------------------------------------------------------------

#[test]
fn cov_oneof_emits_field_children() {
    let r = extract("syntax = \"proto3\";\nmessage Notification {\n  oneof payload {\n    string text = 1;\n    int32 code = 2;\n  }\n}");
    // oneof scope itself is extracted as a Field
    let oneof = r.symbols.iter().find(|s| s.name == "payload" && s.kind == SymbolKind::Field);
    assert!(oneof.is_some(), "expected Field 'payload' for oneof scope; got: {:?}", r.symbols);
    // oneof fields
    let text_field = r.symbols.iter().find(|s| s.name == "text" && s.kind == SymbolKind::Field);
    assert!(text_field.is_some(), "expected Field 'text' inside oneof; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// extend → SymbolKind::Class + TypeRef to extended type
// ---------------------------------------------------------------------------

#[test]
fn cov_extend_emits_class_and_type_ref() {
    let r = extract("syntax = \"proto2\";\nextend google.protobuf.FieldOptions {\n  optional string my_option = 50000;\n}");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Class && s.name.contains("extend"));
    assert!(sym.is_some(), "expected Class for extend block; got: {:?}", r.symbols);
    let has_ref = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name.contains("FieldOptions"));
    assert!(has_ref, "expected TypeRef for extended type; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// nested message → qualified Field children from parent scope
// ---------------------------------------------------------------------------

#[test]
fn cov_nested_message_emits_struct() {
    let r = extract("syntax = \"proto3\";\nmessage Outer {\n  message Inner {\n    string value = 1;\n  }\n  Inner data = 1;\n}");
    let outer = r.symbols.iter().find(|s| s.name == "Outer" && s.kind == SymbolKind::Struct);
    assert!(outer.is_some(), "expected Struct 'Outer'; got: {:?}", r.symbols);
    let inner = r.symbols.iter().find(|s| s.name == "Inner" && s.kind == SymbolKind::Struct);
    assert!(inner.is_some(), "expected nested Struct 'Inner'; got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// Package-qualified names: top-level and nested messages get FQN as
// `<package>.<Name>` and `<package>.<Outer>.<Inner>` respectively. Without
// this, cross-file message refs like `opentelemetry.proto.common.v1.KeyValue`
// fail to resolve via SymbolLookup::by_qualified_name.
// ---------------------------------------------------------------------------

#[test]
fn cov_top_level_message_qualified_with_package() {
    let r = extract("syntax = \"proto3\";\npackage opentelemetry.proto.common.v1;\nmessage KeyValue {\n  string key = 1;\n}");
    let sym = r.symbols.iter().find(|s| s.name == "KeyValue" && s.kind == SymbolKind::Struct);
    assert!(sym.is_some(), "expected Struct 'KeyValue'; got: {:?}", r.symbols.iter().map(|s|&s.name).collect::<Vec<_>>());
    assert_eq!(
        sym.unwrap().qualified_name,
        "opentelemetry.proto.common.v1.KeyValue",
        "top-level message qualified_name must include package; got: {:?}",
        sym.unwrap().qualified_name,
    );
}

#[test]
fn cov_nested_message_qualified_with_package_and_parent() {
    let r = extract("syntax = \"proto3\";\npackage app.api;\nmessage Outer {\n  message Inner {\n    string value = 1;\n  }\n}");
    let inner = r.symbols.iter().find(|s| s.name == "Inner" && s.kind == SymbolKind::Struct);
    assert!(inner.is_some(), "expected nested Struct 'Inner'; got: {:?}", r.symbols.iter().map(|s|&s.name).collect::<Vec<_>>());
    assert_eq!(
        inner.unwrap().qualified_name,
        "app.api.Outer.Inner",
        "nested message qualified_name must include package + parent; got: {:?}",
        inner.unwrap().qualified_name,
    );
}

#[test]
fn cov_enum_qualified_with_package() {
    let r = extract("syntax = \"proto3\";\npackage app.api;\nenum Status {\n  UNKNOWN = 0;\n}");
    let sym = r.symbols.iter().find(|s| s.name == "Status" && s.kind == SymbolKind::Enum);
    assert!(sym.is_some(), "expected Enum 'Status'; got: {:?}", r.symbols.iter().map(|s|&s.name).collect::<Vec<_>>());
    assert_eq!(sym.unwrap().qualified_name, "app.api.Status");
}

#[test]
fn cov_service_qualified_with_package() {
    let r = extract("syntax = \"proto3\";\npackage app.api;\nservice Greeter {\n  rpc SayHello (HelloRequest) returns (HelloReply);\n}");
    let sym = r.symbols.iter().find(|s| s.name == "Greeter" && s.kind == SymbolKind::Interface);
    assert!(sym.is_some(), "expected Interface 'Greeter'; got: {:?}", r.symbols.iter().map(|s|&s.name).collect::<Vec<_>>());
    assert_eq!(sym.unwrap().qualified_name, "app.api.Greeter");
}

#[test]
fn cov_message_without_package_keeps_bare_qname() {
    let r = extract("syntax = \"proto3\";\nmessage Standalone {\n  string x = 1;\n}");
    let sym = r.symbols.iter().find(|s| s.name == "Standalone" && s.kind == SymbolKind::Struct);
    assert!(sym.is_some());
    assert_eq!(sym.unwrap().qualified_name, "Standalone");
}
