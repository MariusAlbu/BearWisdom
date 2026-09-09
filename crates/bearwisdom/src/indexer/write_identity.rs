//! Declaration location and lexical-visibility persistence by exact row ID.
use super::*;

/// Write the structural containment edge (`containing_id`) and a declaration
/// location per symbol. `id_by_idx[k]` is the row id of `pf.symbols[k]`.
///
/// Containment is intra-file here: a symbol's `parent_index` points within the
/// same file's symbol vec, so the parent's id is already known. A member whose
/// parent lives in another file (a Rust `impl` method, a C# partial member
/// declared apart from its class) keeps `containing_id` NULL until the
/// survivor-matching pass can resolve it through the global key→id map.
pub(super) fn write_containment_and_locations(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    pf: &ParsedFile,
    id_by_idx: &[i64],
) -> Result<()> {
    // One declaration location per symbol (mergeable multi-location merging is
    // a survivor-matching concern; on the churn path each file owns its row).
    {
        const LOC_COLS: usize = 4;
        const LOC_BATCH_ROWS: usize = 256;
        let total = id_by_idx.len();
        let mut start = 0;
        while start < total {
            let end = (start + LOC_BATCH_ROWS).min(total);
            let rows = end - start;
            let mut sql = String::with_capacity(96 + rows * 12);
            sql.push_str(
                "INSERT OR IGNORE INTO symbol_locations (symbol_id, file_id, line, col) VALUES ",
            );
            for i in 0..rows {
                if i > 0 {
                    sql.push(',');
                }
                sql.push_str("(?,?,?,?)");
            }
            let mut params: Vec<Value> = Vec::with_capacity(rows * LOC_COLS);
            for i in start..end {
                let sym = &pf.symbols[i];
                params.push(Value::Integer(id_by_idx[i]));
                params.push(Value::Integer(file_id));
                params.push(Value::Integer(sym.start_line as i64));
                params.push(Value::Integer(sym.start_col as i64));
            }
            tx.prepare_cached(&sql)
                .context("Failed to prepare symbol_locations insert")?
                .execute(rusqlite::params_from_iter(params.iter()))
                .context("Failed to insert symbol_locations")?;
            start = end;
        }
    }

    // Containment edge for the symbols that have an in-file parent.
    let mut upd = tx
        .prepare_cached("UPDATE symbols SET containing_id = ?1 WHERE id = ?2")
        .context("Failed to prepare containing_id update")?;
    for (i, sym) in pf.symbols.iter().enumerate() {
        if let Some(p) = sym.parent_index {
            if p < id_by_idx.len() {
                upd.execute(rusqlite::params![id_by_idx[p], id_by_idx[i]])?;
            }
        }
    }
    Ok(())
}

pub(super) fn write_visibility(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    file: &ParsedFile,
    rows: &[i64],
) -> Result<()> {
    crate::db::lexical_visibility::replace(
        tx,
        file_id,
        private_rows(file.flow.lexical.as_ref(), rows),
    )?;
    Ok(())
}

fn private_rows<'a>(
    graph: Option<&'a crate::indexer::lexical::LexicalBindings>,
    rows: &'a [i64],
) -> impl Iterator<Item = i64> + 'a {
    graph.into_iter().flat_map(|graph| {
        graph.lexical_only.iter().filter_map(|binding| {
            graph
                .symbol_slots
                .get(binding)
                .copied()
                .flatten()
                .and_then(|slot| rows.get(slot).copied())
                .filter(|&id| id != 0)
        })
    })
}

#[cfg(test)]
#[path = "write_identity_tests.rs"]
mod tests;
