// =============================================================================
// indexer/resolve/engine/index/augment_tests.rs — augment_from_db type-info
//
// Covers the externals return_type/field_type survival path: an external
// symbol persisted with a signature must yield its return_type / field_type
// when re-loaded via `augment_from_db`, so a chain rooted on an external
// receiver projects the returned type's member. Without that derivation the
// DB-loaded external method has only a name/qname entry and the chain walker's
// `yield_type_of` fallback (`return_type_name`) finds nothing.
// =============================================================================

use std::collections::HashMap;

use crate::db::Database;
use crate::indexer::resolve::engine::{SymbolIndex, SymbolLookup};

/// Insert a `files` row, returning its id.
fn insert_file(db: &Database, path: &str, language: &str, origin: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed, origin)
             VALUES (?1, '', ?2, 0, ?3)",
            rusqlite::params![path, language, origin],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

/// Insert a `symbols` row, returning its id.
fn insert_symbol(
    db: &Database,
    file_id: i64,
    name: &str,
    qname: &str,
    kind: &str,
    scope_path: Option<&str>,
    signature: Option<&str>,
) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO symbols
                (file_id, name, qualified_name, kind, line, col, scope_path, signature, origin)
             VALUES (?1, ?2, ?3, ?4, 1, 0, ?5, ?6, 'external')",
            rusqlite::params![file_id, name, qname, kind, scope_path, signature],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

/// An external method symbol loaded from the DB carries a signature whose
/// return type is parseable. After `augment_from_db` the index must expose
/// that return type via `return_type_name`, so the chain walker's
/// `yield_type_of` fallback can project the returned type's member.
#[test]
fn db_loaded_external_method_signature_yields_return_type() {
    let db = Database::open_in_memory().unwrap();
    let fid = insert_file(&db, "ext:ts:@scope/repo/index.d.ts", "typescript", "external");
    // `findUser(): User` — colon-form return the signature parser reads.
    insert_symbol(
        &db,
        fid,
        "findUser",
        "scope.Repo.findUser",
        "method",
        Some("scope.Repo"),
        Some("findUser(): User"),
    );

    let mut index = SymbolIndex::build(&[], &HashMap::new());
    index.augment_from_db(db.conn());

    assert_eq!(
        index.return_type_name("scope.Repo.findUser"),
        Some("User"),
        "external method loaded from DB must expose its signature return type \
         so a chain rooted on its receiver projects the returned type's member"
    );
}

/// An external field symbol loaded from the DB carries a signature whose
/// declared type is parseable. After `augment_from_db` the index must expose
/// that type via `field_type_name` so a chain rooted on the receiver advances
/// to the field's type for the next member lookup.
#[test]
fn db_loaded_external_field_signature_yields_field_type() {
    let db = Database::open_in_memory().unwrap();
    let fid = insert_file(&db, "ext:ts:@scope/repo/index.d.ts", "typescript", "external");
    // `client: PrismaClient` — colon-form declared type.
    insert_symbol(
        &db,
        fid,
        "client",
        "scope.Service.client",
        "property",
        Some("scope.Service"),
        Some("client: PrismaClient"),
    );

    let mut index = SymbolIndex::build(&[], &HashMap::new());
    index.augment_from_db(db.conn());

    assert_eq!(
        index.field_type_name("scope.Service.client"),
        Some("PrismaClient"),
        "external field loaded from DB must expose its signature declared type"
    );
}
