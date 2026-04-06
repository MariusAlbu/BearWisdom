// =============================================================================
// query/references.rs  —  find-all-references query
//
// "Who uses this symbol?"
//
// Returns all edges that point TO the given symbol (incoming edges).
// For each edge we return the referencing symbol name, file, line, and
// edge kind so the caller can display a proper reference list.
// =============================================================================

use crate::db::Database;
use crate::query::QueryResult;
use crate::types::ReferenceResult;
use anyhow::Context;

/// Find all symbols that reference `target_name`.
///
/// `target_name` may be a simple name or a fully qualified name.
/// When it is a simple name, all symbols with that name are searched
/// (returns references to all overloads / same-named symbols).
///
/// `limit`: maximum number of results (0 = unlimited).
pub fn find_references(db: &Database, target_name: &str, limit: usize) -> QueryResult<Vec<ReferenceResult>> {
    let _timer = db.timer("find_references");

    // Check cache first.
    if let Some(ref cache) = db.query_cache {
        if let Some(cached) = cache.get_references(target_name) {
            if let Ok(result) = serde_json::from_str::<Vec<ReferenceResult>>(&cached) {
                return Ok(result);
            }
        }
    }

    let conn = db.conn();

    // Resolve the target name to one or more symbol IDs.
    let target_ids: Vec<i64> = {
        if target_name.contains('.') {
            // Qualified name — exact match.
            let mut stmt = conn.prepare(
                "SELECT id FROM symbols WHERE qualified_name = ?1"
            ).context("Failed to prepare qualified target lookup")?;
            let rows = stmt.query_map([target_name], |r| r.get(0))
                .context("Failed to query qualified target")?;
            rows.filter_map(|r| r.ok()).collect()
        } else {
            // Simple name — all symbols with that name.
            let mut stmt = conn.prepare(
                "SELECT id FROM symbols WHERE name = ?1"
            ).context("Failed to prepare simple target lookup")?;
            let rows = stmt.query_map([target_name], |r| r.get(0))
                .context("Failed to query simple target")?;
            rows.filter_map(|r| r.ok()).collect()
        }
    };

    if target_ids.is_empty() {
        return Ok(vec![]);
    }

    let mut results: Vec<ReferenceResult> = Vec::new();

    for target_id in &target_ids {
        // Find all edges pointing to this target.
        let limit_clause = if limit > 0 {
            format!("LIMIT {limit}")
        } else {
            String::new()
        };

        let sql = format!(
            "SELECT src.name, src.kind, f.path, e.source_line, e.kind, e.confidence
             FROM edges e
             JOIN symbols src ON e.source_id = src.id
             JOIN files   f   ON src.file_id  = f.id
             WHERE e.target_id = ?1
             ORDER BY f.path, e.source_line
             {limit_clause}"
        );

        let mut stmt = conn.prepare(&sql).context("Failed to prepare references query")?;
        let rows = stmt.query_map([target_id], |row| {
            Ok(ReferenceResult {
                referencing_symbol: row.get(0)?,
                referencing_kind: row.get(1)?,
                file_path: row.get(2)?,
                line: row.get::<_, Option<u32>>(3)?.unwrap_or(0),
                edge_kind: row.get(4)?,
                confidence: row.get(5)?,
            })
        }).context("Failed to execute references query")?;

        for row in rows {
            results.push(row?);
        }
    }

    // Sort by file path then line for stable output.
    results.sort_by(|a, b| {
        a.file_path.cmp(&b.file_path)
            .then(a.line.cmp(&b.line))
    });

    if limit > 0 && results.len() > limit {
        results.truncate(limit);
    }

    // Store in cache.
    if let Some(ref cache) = db.query_cache {
        if let Ok(json) = serde_json::to_string(&results) {
            cache.put_references(target_name.to_string(), json);
        }
    }

    Ok(results)
}

/// JSON-returning variant of [`find_references`] for use in MCP and CLI paths.
///
/// Returns the raw cached JSON string on a cache hit, skipping the
/// deserialize → struct → reserialize roundtrip that occurs when the caller
/// would otherwise call `find_references` and then `serde_json::to_string`.
pub fn find_references_json(
    db: &Database,
    target_name: &str,
    limit: usize,
) -> super::QueryResult<String> {
    // Raw cache hit: return JSON directly without deserializing.
    if let Some(ref cache) = db.query_cache {
        if let Some(raw) = cache.get_references_raw(target_name) {
            return Ok(raw);
        }
    }
    let result = find_references(db, target_name, limit)?;
    serde_json::to_string(&result)
        .map_err(|e| super::QueryError::Internal(anyhow::anyhow!("serialization error: {e}")))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "references_tests.rs"]
mod tests;
