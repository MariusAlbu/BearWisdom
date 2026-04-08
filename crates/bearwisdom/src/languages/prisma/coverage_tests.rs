// =============================================================================
// prisma/coverage_tests.rs
//
// Node-kind coverage for PrismaPlugin::symbol_node_kinds() and ref_node_kinds().
// Grammar returns None; extraction is performed by the line scanner.
//
// symbol_node_kinds: model_declaration, enum_declaration,
//                   datasource_declaration, generator_declaration,
//                   type_declaration
// ref_node_kinds:    column_declaration, enumeral
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_model_declaration_produces_struct() {
    let src = "model User {\n  id Int @id\n  name String\n}";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Struct && s.name == "User"),
        "model should produce Struct(User); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_model_field_produces_field() {
    let src = "model Post {\n  id Int @id\n  title String\n}";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Field && s.name == "title"),
        "model field should produce Field(title); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_enum_declaration_produces_enum() {
    let src = "enum Role {\n  ADMIN\n  USER\n}";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Enum && s.name == "Role"),
        "enum should produce Enum(Role); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_relation_field_produces_type_ref() {
    // A field whose type references another model → TypeRef
    let src = "model Post {\n  id Int @id\n  author User @relation(fields: [authorId], references: [id])\n  authorId Int\n}\nmodel User {\n  id Int @id\n}";
    let r = extract::extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "User"),
        "relation field should produce TypeRef(User); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// view_declaration → SymbolKind::Class
// ---------------------------------------------------------------------------

#[test]
fn cov_view_declaration_produces_class() {
    let src = "view ActiveUsers {\n  id Int\n  email String\n}";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "ActiveUsers"),
        "view should produce Class(ActiveUsers); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// datasource_declaration → SymbolKind::Variable
// ---------------------------------------------------------------------------

#[test]
fn cov_datasource_declaration_produces_variable() {
    let src = "datasource db {\n  provider = \"postgresql\"\n  url      = env(\"DATABASE_URL\")\n}";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "db"),
        "datasource should produce Variable(db); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// generator_declaration → SymbolKind::Variable
// ---------------------------------------------------------------------------

#[test]
fn cov_generator_declaration_produces_variable() {
    let src = "generator client {\n  provider = \"prisma-client-js\"\n}";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "client"),
        "generator should produce Variable(client); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// type_declaration → SymbolKind::TypeAlias
// ---------------------------------------------------------------------------

#[test]
fn cov_type_declaration_produces_type_alias() {
    let src = "type Address {\n  street String\n  city   String\n}";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::TypeAlias && s.name == "Address"),
        "type block should produce TypeAlias(Address); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// enumeral → SymbolKind::EnumMember (child of enum scope)
// ---------------------------------------------------------------------------

#[test]
fn cov_enumeral_produces_enum_member() {
    let src = "enum Status {\n  PENDING\n  ACTIVE\n  CANCELLED\n}";
    let r = extract::extract(src);
    let members: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::EnumMember)
        .map(|s| s.name.as_str())
        .collect();
    for expected in &["PENDING", "ACTIVE", "CANCELLED"] {
        assert!(members.contains(expected), "expected EnumMember '{expected}'; got: {members:?}");
    }
}

// ---------------------------------------------------------------------------
// Scalar field types — no TypeRef emitted for Prisma built-ins
// ---------------------------------------------------------------------------

#[test]
fn cov_scalar_field_types_no_type_ref() {
    let src = "model Item {\n  id      Int     @id\n  label   String\n  price   Float\n  active  Boolean\n  created DateTime\n}";
    let r = extract::extract(src);
    // No TypeRef should be emitted for any Prisma scalar
    let scalar_refs: Vec<&str> = r.refs.iter()
        .filter(|rf| rf.kind == EdgeKind::TypeRef)
        .map(|rf| rf.target_name.as_str())
        .filter(|name| ["Int", "String", "Float", "Boolean", "DateTime"].contains(name))
        .collect();
    assert!(scalar_refs.is_empty(), "should not emit TypeRef for Prisma scalars; got: {scalar_refs:?}");
}

// ---------------------------------------------------------------------------
// Optional field (?) — Field still emitted, type still resolved
// ---------------------------------------------------------------------------

#[test]
fn cov_optional_field_emits_field() {
    let src = "model Profile {\n  id  Int     @id\n  bio String?\n}";
    let r = extract::extract(src);
    let field = r.symbols.iter().find(|s| s.name == "bio" && s.kind == SymbolKind::Field);
    assert!(field.is_some(), "expected Field 'bio' for optional field; got: {:?}", r.symbols);
    // String? → base type String is a scalar, so no TypeRef
    let has_string_ref = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "String");
    assert!(!has_string_ref, "should not emit TypeRef for optional scalar 'String?'; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// Array field ([]) — Field emitted, TypeRef for non-scalar array type
// ---------------------------------------------------------------------------

#[test]
fn cov_array_field_non_scalar_emits_type_ref() {
    let src = "model User {\n  id   Int    @id\n  tags Tag[]\n}\nmodel Tag {\n  id   Int    @id\n}";
    let r = extract::extract(src);
    let field = r.symbols.iter().find(|s| s.name == "tags" && s.kind == SymbolKind::Field);
    assert!(field.is_some(), "expected Field 'tags' for array field; got: {:?}", r.symbols);
    let has_ref = r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "Tag");
    assert!(has_ref, "expected TypeRef to 'Tag' from array field; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// doc comment (///) — captured on parent symbol
// ---------------------------------------------------------------------------

#[test]
fn cov_doc_comment_captured_on_model() {
    let src = "/// A user account in the system.\nmodel Account {\n  id Int @id\n}";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Account" && s.kind == SymbolKind::Struct);
    assert!(sym.is_some(), "expected Struct 'Account'; got: {:?}", r.symbols);
    let doc = sym.unwrap().doc_comment.as_deref().unwrap_or("");
    assert!(
        doc.contains("user account"),
        "expected doc comment to contain 'user account'; got: {:?}",
        doc
    );
}

// ---------------------------------------------------------------------------
// Multiple models in one file — all emitted
// ---------------------------------------------------------------------------

#[test]
fn cov_multiple_models_all_emitted() {
    let src = "model User {\n  id Int @id\n}\nmodel Post {\n  id Int @id\n}\nmodel Comment {\n  id Int @id\n}";
    let r = extract::extract(src);
    let structs: Vec<&str> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Struct)
        .map(|s| s.name.as_str())
        .collect();
    for expected in &["User", "Post", "Comment"] {
        assert!(structs.contains(expected), "expected Struct '{expected}'; got: {structs:?}");
    }
}
