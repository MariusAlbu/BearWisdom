//! Resolution evidence schema and additive upgrades for existing indexes.

use rusqlite::Connection;

pub(super) fn create(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(include_str!("resolution_schema.sql"))?;
    let has_byte_offset: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('ref_resolutions') WHERE name = 'source_byte')",
        [], |row| row.get(0),
    )?;
    if !has_byte_offset {
        // NULL means unavailable evidence. Zero is a valid source position.
        conn.execute(
            "ALTER TABLE ref_resolutions ADD COLUMN source_byte INTEGER",
            [],
        )?;
    }
    let has_selector: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('ref_resolutions') WHERE name = 'source_selector_byte')",
        [], |row| row.get(0),
    )?;
    if !has_selector {
        conn.execute(
            "ALTER TABLE ref_resolutions ADD COLUMN source_selector_byte INTEGER",
            [],
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "resolution_schema_tests.rs"]
mod tests;
