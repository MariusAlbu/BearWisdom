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
