//! Persist declarations that may only be reached through a lexical binding.
//! This is scope metadata, not accessibility flags or a name exclusion list.
use rusqlite::Connection;

pub(crate) fn create(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS lexical_only_symbols (
        symbol_id INTEGER PRIMARY KEY REFERENCES symbols(id) ON DELETE CASCADE
    );",
    )
}

pub(crate) fn replace(
    conn: &Connection,
    file_id: i64,
    ids: impl IntoIterator<Item = i64>,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM lexical_only_symbols WHERE symbol_id IN
        (SELECT id FROM symbols WHERE file_id = ?1)",
        [file_id],
    )?;
    let mut insert =
        conn.prepare_cached("INSERT OR IGNORE INTO lexical_only_symbols (symbol_id) VALUES (?1)")?;
    for id in ids {
        insert.execute([id])?;
    }
    Ok(())
}

pub(crate) fn read(conn: &Connection) -> rusqlite::Result<Vec<i64>> {
    conn.prepare("SELECT symbol_id FROM lexical_only_symbols")?
        .query_map([], |row| row.get(0))?
        .collect()
}

#[cfg(test)]
#[path = "lexical_visibility_tests.rs"]
mod tests;
