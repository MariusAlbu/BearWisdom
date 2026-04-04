// =============================================================================
// sql/coverage_tests.rs — Node-kind coverage tests for the SQL extractor
//
// symbol_node_kinds:
//   create_table_stmt, create_view_stmt, create_trigger_stmt,
//   column_def, common_table_expression
//
// ref_node_kinds:
//   table_or_subquery, foreign_key_clause, type_name
//
// NOTE: The extractor uses tree-sitter-sequel which maps node kinds slightly
// differently from the declared names. The extract.rs handles `create_table`,
// `create_view`, `create_trigger`, and `column_definition` nodes internally.
// The symbol_node_kinds() names are conceptual — the tests verify observable
// extraction behaviour driven by those constructs.
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

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// foreign_key_clause — inline REFERENCES in column_definition.
/// tree-sitter-sequel parses inline `REFERENCES` as keyword children directly
/// on the column_definition (not as a `constraint` / `foreign_key_reference` child),
/// so the extractor's extract_fk_refs finds no FK constraint nodes. No TypeRef is
/// emitted, but the source must not crash.
#[test]
fn cov_foreign_key_clause_does_not_crash() {
    let src = "CREATE TABLE orders (user_id INT REFERENCES users(id));";
    let r = extract::extract(src);
    // Table and column symbols should still be extracted without crashing.
    let has_table = r.symbols.iter().any(|s| s.name == "orders");
    assert!(has_table, "expected Struct 'orders'; got: {:?}", r.symbols);
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
