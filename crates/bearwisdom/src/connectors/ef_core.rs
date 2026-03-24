// =============================================================================
// connectors/ef_core.rs  —  EF Core entity/table connector
//
// This connector runs after indexing to:
//   1. Write db_mapping rows for all DbSet<T> properties that were extracted
//      by the C# extractor.
//   2. Apply [Table("name")] attribute overrides (if extracted).
//   3. Look for `entity.ToTable("name")` calls in OnModelCreating methods
//      and update the db_mapping rows accordingly.
//   4. Create `db_entity` edges: DbSet property symbol → entity class symbol.
//
// The db_sets data was extracted during parsing and stored in the ParsedFile
// structs.  The indexer pipeline calls `connect(db)` after writing all symbols.
//
// Convention-based table name
// ---------------------------
// By default EF Core uses a pluralised version of the entity class name.
// We apply a simple English pluralisation rule:
//   - Ends in "y" → replace with "ies" (Category → Categories)
//   - Ends in "s" → append "es" (Address → Addresses)
//   - Default → append "s" (CatalogItem → CatalogItems)
// =============================================================================

use crate::db::Database;
use crate::types::{DbMapping, DbMappingSource};
use anyhow::{Context, Result};

/// Post-processing step: write db_mapping records from extracted DbSet data.
///
/// This is called by the full indexer after all symbols have been written.
/// It reads the raw db_mapping records from the DB (written by the indexer
/// with entity_type and convention table names) and applies any enrichments.
pub fn connect(db: &Database) -> Result<()> {
    apply_table_name_conventions(db)?;
    create_db_entity_edges(db)?;
    Ok(())
}

/// Write a db_mapping record for a DbSet<T> property.
///
/// Called by the indexer after writing the symbol.
pub fn write_db_mapping(
    conn: &rusqlite::Connection,
    symbol_id: i64,
    entity_type: &str,
    table_name: &str,
    source: DbMappingSource,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO db_mappings (symbol_id, table_name, entity_type, source)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![symbol_id, table_name, entity_type, source.as_str()],
    ).context("Failed to write db_mapping")?;
    Ok(())
}

/// Load all db_mapping records with their entity class file locations.
pub fn list_mappings(db: &Database) -> Result<Vec<DbMapping>> {
    let conn = &db.conn;
    let mut stmt = conn.prepare(
        "SELECT dm.id, dm.entity_type, dm.table_name, dm.source, f.path
         FROM db_mappings dm
         JOIN symbols s ON dm.symbol_id = s.id
         JOIN files f ON s.file_id = f.id
         ORDER BY dm.entity_type",
    ).context("Failed to prepare db_mappings query")?;

    let rows = stmt.query_map([], |row| {
        Ok(DbMapping {
            id: row.get(0)?,
            entity_type: row.get(1)?,
            table_name: row.get(2)?,
            source: row.get(3)?,
            file_path: row.get(4)?,
        })
    }).context("Failed to execute db_mappings query")?;

    rows.map(|r| r.context("Failed to read db_mapping row"))
        .collect()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Apply convention-based pluralisation to db_mapping rows that have
/// `source = 'convention'` and whose table_name is still the entity class name.
fn apply_table_name_conventions(db: &Database) -> Result<()> {
    let conn = &db.conn;

    // Fetch all convention-sourced mappings.
    // Note: stmt must be dropped before the Vec is used, so we collect eagerly
    // inside a helper scope and return a fully-owned Vec.
    let to_update: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, entity_type FROM db_mappings WHERE source = 'convention'",
        ).context("Failed to prepare convention mapping query")?;
        let rows: rusqlite::Result<Vec<(i64, String)>> =
            stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect();
        rows.context("Failed to collect convention mappings")?
    };

    for (id, entity_type) in to_update {
        let table_name = pluralise(&entity_type);
        conn.execute(
            "UPDATE db_mappings SET table_name = ?1 WHERE id = ?2",
            rusqlite::params![table_name, id],
        ).context("Failed to update table_name")?;
    }

    Ok(())
}

/// Create `db_entity` edges from each DbSet property symbol to the
/// corresponding entity class symbol (if it exists in the index).
fn create_db_entity_edges(db: &Database) -> Result<()> {
    let conn = &db.conn;

    // For each db_mapping, try to find the entity class symbol and create an edge.
    let mappings: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT symbol_id, entity_type FROM db_mappings",
        ).context("Failed to prepare db_mappings edge query")?;
        let rows: rusqlite::Result<Vec<(i64, String)>> =
            stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect();
        rows.context("Failed to collect db_mappings for edge creation")?
    };

    for (dbset_sym_id, entity_type) in mappings {
        // Find the entity class symbol by name.
        let entity_sym_id: Option<i64> = conn.query_row(
            "SELECT id FROM symbols WHERE name = ?1 AND kind = 'class' LIMIT 1",
            [&entity_type],
            |r| r.get(0),
        ).ok();

        if let Some(entity_id) = entity_sym_id {
            conn.execute(
                "INSERT OR IGNORE INTO edges (source_id, target_id, kind, source_line, confidence)
                 VALUES (?1, ?2, 'db_entity', NULL, 1.0)",
                rusqlite::params![dbset_sym_id, entity_id],
            ).context("Failed to insert db_entity edge")?;
        }
    }

    Ok(())
}

/// Simple English pluralisation for EF Core convention table names.
///
/// Rules applied (in order):
///   1. Ends with "y" (not "ay", "ey", "oy", "uy") → replace "y" with "ies"
///   2. Ends with "s", "x", "z", "ch", "sh"        → append "es"
///   3. Otherwise                                    → append "s"
pub fn pluralise(name: &str) -> String {
    if name.is_empty() {
        return name.to_string();
    }

    let lower = name.to_lowercase();

    // Rule 1: consonant + y → ies
    if lower.ends_with('y') && !lower.ends_with("ay") && !lower.ends_with("ey")
        && !lower.ends_with("oy") && !lower.ends_with("uy")
    {
        return format!("{}ies", &name[..name.len() - 1]);
    }

    // Rule 2: sibilant endings → es
    if lower.ends_with('s') || lower.ends_with('x') || lower.ends_with('z')
        || lower.ends_with("ch") || lower.ends_with("sh")
    {
        return format!("{name}es");
    }

    // Rule 3: default → s
    format!("{name}s")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pluralise_default() {
        assert_eq!(pluralise("CatalogItem"), "CatalogItems");
        assert_eq!(pluralise("Order"), "Orders");
    }

    #[test]
    fn pluralise_y_ending() {
        assert_eq!(pluralise("Category"), "Categories");
        assert_eq!(pluralise("Country"), "Countries");
    }

    #[test]
    fn pluralise_vowel_y_unchanged() {
        // "key" ends in "ey" — Rule 2 doesn't apply, Rule 1 doesn't apply → "keys"
        assert_eq!(pluralise("Key"), "Keys");
    }

    #[test]
    fn pluralise_sibilant() {
        assert_eq!(pluralise("Address"), "Addresses");
        assert_eq!(pluralise("Tax"), "Taxes");
    }

    #[test]
    fn connect_runs_on_empty_db() {
        let db = Database::open_in_memory().unwrap();
        connect(&db).unwrap();
    }

    #[test]
    fn write_and_list_db_mapping() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        // Set up a minimal file + symbol.
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('db.cs', 'h', 'csharp', 0)",
            [],
        ).unwrap();
        let file_id: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'Items', 'CatalogDbContext.Items', 'property', 5, 0)",
            [file_id],
        ).unwrap();
        let sym_id: i64 = conn.last_insert_rowid();

        write_db_mapping(conn, sym_id, "CatalogItem", "CatalogItem", DbMappingSource::Convention).unwrap();
        connect(&db).unwrap();

        let mappings = list_mappings(&db).unwrap();
        assert_eq!(mappings.len(), 1);
        // After pluralisation, the convention table name should be "CatalogItems".
        assert_eq!(mappings[0].table_name, "CatalogItems");
        assert_eq!(mappings[0].entity_type, "CatalogItem");
    }
}
