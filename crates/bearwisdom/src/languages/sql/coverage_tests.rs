// =============================================================================
// sql/coverage_tests.rs — Node-kind coverage tests for the SQL extractor
//
// symbol_node_kinds (actual tree-sitter-sequel 0.3.x kinds):
//   create_table, create_view, create_index, create_function, column_definition, cte
//
// ref_node_kinds:
//   object_reference  (covers FK REFERENCES, ALTER TABLE target, view FROM targets)
//
// tree-sitter-sequel notes:
//   - CREATE TRIGGER produces an ERROR node; extractor should not crash.
//   - FK inline REFERENCES: keyword_references + object_reference directly under
//     column_definition (no constraint/foreign_key_reference wrapper).
//   - CREATE INDEX name is a bare identifier, not object_reference.
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};


// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

/// create_table → SymbolKind::Struct
#[test]
fn cov_create_table_emits_struct() {
    let r = extract::extract("CREATE TABLE users (id INT);");
    let sym = r.symbols.iter().find(|s| s.name == "users");
    assert!(sym.is_some(), "expected Struct 'users'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Struct);
}

/// column_def (column_definition) → SymbolKind::Field inside the table
#[test]
fn cov_column_def_emits_field() {
    let r = extract::extract("CREATE TABLE orders (id INT, total DECIMAL);");
    let id_field = r.symbols.iter().find(|s| s.name == "id" && s.kind == SymbolKind::Field);
    assert!(id_field.is_some(), "expected Field 'id'; got: {:?}", r.symbols);
    let total_field = r.symbols.iter().find(|s| s.name == "total" && s.kind == SymbolKind::Field);
    assert!(total_field.is_some(), "expected Field 'total'; got: {:?}", r.symbols);
}

/// create_view → SymbolKind::Class
#[test]
fn cov_create_view_emits_class() {
    let r = extract::extract("CREATE VIEW active_users AS SELECT * FROM users WHERE active = 1;");
    let sym = r.symbols.iter().find(|s| s.name == "active_users");
    assert!(sym.is_some(), "expected Class 'active_users' from CREATE VIEW; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Class);
}

/// create_trigger_stmt — the tree-sitter-sequel grammar does not fully support
/// CREATE TRIGGER syntax and produces an ERROR node. The extractor must not crash.
#[test]
fn cov_create_trigger_does_not_crash() {
    let src = "CREATE TRIGGER update_ts BEFORE UPDATE ON users BEGIN END;";
    let r = extract::extract(src);
    // Grammar produces an error node; no Function symbol is expected but no panic.
    let _ = r;
}

/// common_table_expression — CTE inside a query; extractor may not emit a
/// dedicated symbol but should not crash.
#[test]
fn cov_common_table_expression_does_not_crash() {
    let src = "WITH cte AS (SELECT id FROM users) SELECT * FROM cte;";
    let r = extract::extract(src);
    let _ = r;
}

/// create_index → SymbolKind::Variable + TypeRef to the indexed table
#[test]
fn cov_create_index_emits_variable_and_ref() {
    let r = extract::extract("CREATE INDEX idx_name ON users (name);");
    let sym = r.symbols.iter().find(|s| s.name == "idx_name");
    assert!(sym.is_some(), "expected Variable 'idx_name' from CREATE INDEX; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, crate::types::SymbolKind::Variable);
    let table_ref = r.refs.iter().find(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "users");
    assert!(table_ref.is_some(), "expected TypeRef to 'users' from CREATE INDEX; got: {:?}", r.refs);
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// object_reference — inline REFERENCES in column_definition emits a TypeRef.
/// tree-sitter-sequel emits keyword_references + object_reference directly under
/// column_definition; the extractor detects this and emits a TypeRef edge.
#[test]
fn cov_foreign_key_emits_type_ref() {
    let src = "CREATE TABLE orders (user_id INT REFERENCES users(id));";
    let r = extract::extract(src);
    // Table symbol should be extracted.
    let has_table = r.symbols.iter().any(|s| s.name == "orders");
    assert!(has_table, "expected Struct 'orders'; got: {:?}", r.symbols);
    // FK reference should produce a TypeRef to 'users'.
    let fk_ref = r.refs.iter().find(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "users");
    assert!(fk_ref.is_some(), "expected TypeRef to 'users' from FK REFERENCES; got: {:?}", r.refs);
}

/// table_or_subquery — table reference in a query; extractor should handle
/// it without crashing (ALTER TABLE generates TypeRef edges for referenced tables).
#[test]
fn cov_table_or_subquery_does_not_crash() {
    let src = "ALTER TABLE orders ADD COLUMN note TEXT;";
    let r = extract::extract(src);
    // ALTER TABLE emits a TypeRef for the referenced table
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"orders"),
        "expected TypeRef to 'orders' from ALTER TABLE; got: {type_refs:?}"
    );
}

/// type_name — custom type column → TypeRef
#[test]
fn cov_type_name_custom_column_does_not_crash() {
    // A column with a custom type (not a SQL keyword) should be gracefully handled.
    let src = "CREATE TABLE items (data jsonb);";
    let r = extract::extract(src);
    let _ = r;
}
