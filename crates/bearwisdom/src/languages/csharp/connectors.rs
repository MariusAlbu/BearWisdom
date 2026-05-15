// =============================================================================
// languages/csharp/connectors.rs — EF Core post-index hook
//
// Owns the EF Core post-index helper: applies pluralisation conventions to
// `db_mappings`, then writes `db_entity` edges. DI bindings + integration
// event pairing now run as resolver-side FlowEmissions (DiBinding /
// NamedChannel{EventBus}); the legacy regex-scanning helpers that lived
// here have been removed.
// =============================================================================

use anyhow::Context;

// ===========================================================================
// EF Core post-index hook
// ===========================================================================

/// Post-index enrichment for EF Core: apply convention pluralisation and
/// create `db_entity` edges.
///
/// Called from `CSharpPlugin::post_index()` after all symbols have been written.
pub fn run_ef_core(db: &crate::db::Database) -> anyhow::Result<()> {
    ef_core_connect(db)
}

// ---------------------------------------------------------------------------
// EF Core helpers (inlined from connectors/ef_core.rs)
// ---------------------------------------------------------------------------

use crate::db::Database;
use crate::types::{DbMapping, DbMappingSource};

fn ef_core_connect(db: &Database) -> anyhow::Result<()> {
    ef_apply_table_name_conventions(db)?;
    ef_create_db_entity_edges(db)?;
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
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO db_mappings (symbol_id, table_name, entity_type, source)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![symbol_id, table_name, entity_type, source.as_str()],
    ).context("Failed to write db_mapping")?;
    Ok(())
}

/// Load all db_mapping records with their entity class file locations.
pub fn list_db_mappings(db: &Database) -> anyhow::Result<Vec<DbMapping>> {
    let conn = db.conn();
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

fn ef_apply_table_name_conventions(db: &Database) -> anyhow::Result<()> {
    let conn = db.conn();
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
        let table_name = ef_pluralise(&entity_type);
        conn.execute(
            "UPDATE db_mappings SET table_name = ?1 WHERE id = ?2",
            rusqlite::params![table_name, id],
        ).context("Failed to update table_name")?;
    }
    Ok(())
}

fn ef_create_db_entity_edges(db: &Database) -> anyhow::Result<()> {
    let conn = db.conn();
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
pub fn ef_pluralise(name: &str) -> String {
    if name.is_empty() {
        return name.to_string();
    }
    let lower = name.to_lowercase();
    if lower.ends_with('y') && !lower.ends_with("ay") && !lower.ends_with("ey")
        && !lower.ends_with("oy") && !lower.ends_with("uy")
    {
        return format!("{}ies", &name[..name.len() - 1]);
    }
    if lower.ends_with('s') || lower.ends_with('x') || lower.ends_with('z')
        || lower.ends_with("ch") || lower.ends_with("sh")
    {
        return format!("{name}es");
    }
    format!("{name}s")
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod ef_core_tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn ef_pluralise_default() {
        assert_eq!(ef_pluralise("CatalogItem"), "CatalogItems");
        assert_eq!(ef_pluralise("Order"), "Orders");
    }

    #[test]
    fn ef_pluralise_y_ending() {
        assert_eq!(ef_pluralise("Category"), "Categories");
        assert_eq!(ef_pluralise("Country"), "Countries");
    }

    #[test]
    fn ef_pluralise_vowel_y_unchanged() {
        assert_eq!(ef_pluralise("Key"), "Keys");
    }

    #[test]
    fn ef_pluralise_sibilant() {
        assert_eq!(ef_pluralise("Address"), "Addresses");
        assert_eq!(ef_pluralise("Tax"), "Taxes");
    }

    #[test]
    fn ef_core_connect_runs_on_empty_db() {
        let db = Database::open_in_memory().unwrap();
        ef_core_connect(&db).unwrap();
    }

    #[test]
    fn ef_write_and_list_db_mapping() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

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
        ef_core_connect(&db).unwrap();

        let mappings = list_db_mappings(&db).unwrap();
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].table_name, "CatalogItems");
        assert_eq!(mappings[0].entity_type, "CatalogItem");
    }
}
