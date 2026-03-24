// =============================================================================
// query/blast_radius.rs  —  N-hop dependency impact analysis
//
// "If I change symbol X, what else is affected?"
//
// Uses a recursive CTE (Common Table Expression) to walk the edge graph
// BACKWARDS from the target symbol.  Each hop goes one level up the
// dependency chain: if A calls B calls C, and we change C, then the blast
// radius of C includes B (depth=1) and A (depth=2).
//
// Terminology:
//   • center   — the symbol being changed
//   • affected — every symbol that (transitively) depends on the center
//   • depth    — number of hops from the center (1 = direct caller)
// =============================================================================

use crate::db::Database;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::query::architecture::SymbolSummary;

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// A single symbol that would be affected if the center symbol changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedSymbol {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file_path: String,
    /// How many hops away from the center symbol (1 = direct dependent).
    pub depth: u32,
    /// The kind of edge connecting this symbol to its predecessor in the path.
    pub edge_kind: String,
}

/// The complete blast radius result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlastRadiusResult {
    /// The symbol that was the starting point of the analysis.
    pub center: SymbolSummary,
    /// All symbols reachable within `max_depth` hops, excluding the center.
    pub affected: Vec<AffectedSymbol>,
    /// Total number of affected symbols found (may be < affected.len() if
    /// duplicates were collapsed by DISTINCT).
    pub total_affected: u32,
    /// The maximum depth actually found (may be less than the requested limit).
    pub max_depth: u32,
}

// ---------------------------------------------------------------------------
// Public function
// ---------------------------------------------------------------------------

/// Compute the blast radius of `symbol_name` up to `max_depth` hops.
///
/// `symbol_name` may be a simple name or a fully-qualified name.
/// When the name is ambiguous (multiple symbols share it), the first match
/// by qualified_name alphabetically is used.
///
/// Returns `Ok(None)` if the symbol is not found in the index.
pub fn blast_radius(
    db: &Database,
    symbol_name: &str,
    max_depth: u32,
) -> Result<Option<BlastRadiusResult>> {
    let conn = &db.conn;

    // --- Resolve the symbol to an id + summary ---
    // Try exact qualified name first, then simple name fallback.
    let lookup_sql = if symbol_name.contains('.') {
        "SELECT s.id, s.name, s.qualified_name, s.kind, f.path, s.line
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.qualified_name = ?1
         LIMIT 1"
    } else {
        "SELECT s.id, s.name, s.qualified_name, s.kind, f.path, s.line
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.name = ?1
         ORDER BY s.qualified_name
         LIMIT 1"
    };

    // query_row returns Err(QueryReturnedNoRows) if nothing is found.
    let center_result = conn.query_row(lookup_sql, [symbol_name], |row| {
        Ok((
            row.get::<_, i64>(0)?,        // id
            SymbolSummary {
                name:           row.get(1)?,
                qualified_name: row.get(2)?,
                kind:           row.get(3)?,
                file_path:      row.get(4)?,
                line:           row.get(5)?,
            },
        ))
    });

    let (center_id, center) = match center_result {
        Ok(pair) => pair,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(e).context("Failed to look up center symbol"),
    };

    // --- Recursive CTE: walk backwards through the edge graph ---
    //
    // The CTE starts from the center symbol and follows edges in the
    // REVERSE direction (source → target means "source depends on target",
    // so we follow source_id to find all dependents of the center).
    //
    // DISTINCT on the symbol id prevents infinite loops in cyclic graphs.
    // The WHERE depth < max_depth guard is the termination condition.
    //
    // We also capture the edge kind at the last hop for display.
    let sql = "
        WITH RECURSIVE blast(id, depth, edge_kind) AS (
            -- Seed: the center symbol itself at depth 0 (no edge kind needed).
            SELECT ?1 AS id, 0 AS depth, '' AS edge_kind

            UNION

            -- Recursive step: find every symbol that has an edge pointing TO
            -- a symbol already in our blast set.  That means the source_id
            -- symbol depends on blast.id, so it's affected.
            SELECT e.source_id,
                   blast.depth + 1,
                   e.kind
            FROM edges e
            JOIN blast ON blast.id = e.target_id
            WHERE blast.depth < ?2
        )
        SELECT DISTINCT
               s.name,
               s.qualified_name,
               s.kind,
               f.path        AS file_path,
               b.depth,
               b.edge_kind
        FROM blast b
        JOIN symbols s ON s.id = b.id
        JOIN files   f ON f.id = s.file_id
        WHERE b.depth > 0          -- exclude the center itself
        ORDER BY b.depth, f.path
    ";

    let mut stmt = conn.prepare(sql).context("Failed to prepare blast radius CTE")?;

    let rows = stmt.query_map(rusqlite::params![center_id, max_depth], |row| {
        Ok(AffectedSymbol {
            name:           row.get(0)?,
            qualified_name: row.get(1)?,
            kind:           row.get(2)?,
            file_path:      row.get(3)?,
            depth:          row.get(4)?,
            edge_kind:      row.get(5)?,
        })
    }).context("Failed to execute blast radius query")?;

    let affected: Vec<AffectedSymbol> =
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect blast radius rows")?;

    let total_affected = affected.len() as u32;
    let max_depth_found = affected.iter().map(|a| a.depth).max().unwrap_or(0);

    Ok(Some(BlastRadiusResult {
        center,
        affected,
        total_affected,
        max_depth: max_depth_found,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    /// Minimal setup: one file, multiple symbols and edges.
    fn setup_graph(db: &Database) -> (i64, i64, i64, i64) {
        let conn = &db.conn;
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('a.cs', 'h', 'csharp', 0)",
            [],
        ).unwrap();
        let fid = conn.last_insert_rowid();

        // Graph: D → C → B → A   (A is the center we'll query)
        for (name, qname, line) in [("A", "NS.A", 1i64), ("B", "NS.B", 2), ("C", "NS.C", 3), ("D", "NS.D", 4)] {
            conn.execute(
                "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, ?2, ?3, 'method', ?4, 0)",
                rusqlite::params![fid, name, qname, line],
            ).unwrap();
        }
        let a: i64 = conn.query_row("SELECT id FROM symbols WHERE name='A'", [], |r| r.get(0)).unwrap();
        let b: i64 = conn.query_row("SELECT id FROM symbols WHERE name='B'", [], |r| r.get(0)).unwrap();
        let c: i64 = conn.query_row("SELECT id FROM symbols WHERE name='C'", [], |r| r.get(0)).unwrap();
        let d: i64 = conn.query_row("SELECT id FROM symbols WHERE name='D'", [], |r| r.get(0)).unwrap();

        // B calls A, C calls B, D calls C
        conn.execute("INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)", rusqlite::params![b, a]).unwrap();
        conn.execute("INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)", rusqlite::params![c, b]).unwrap();
        conn.execute("INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)", rusqlite::params![d, c]).unwrap();

        (a, b, c, d)
    }

    #[test]
    fn blast_radius_direct_callers_depth_1() {
        let db = Database::open_in_memory().unwrap();
        setup_graph(&db);

        let result = blast_radius(&db, "A", 1).unwrap().expect("symbol not found");
        assert_eq!(result.center.name, "A");
        // Only B directly calls A.
        assert_eq!(result.affected.len(), 1);
        assert_eq!(result.affected[0].name, "B");
        assert_eq!(result.affected[0].depth, 1);
    }

    #[test]
    fn blast_radius_depth_2_includes_transitive() {
        let db = Database::open_in_memory().unwrap();
        setup_graph(&db);

        let result = blast_radius(&db, "A", 2).unwrap().expect("symbol not found");
        let names: Vec<&str> = result.affected.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"B"), "B should be at depth 1");
        assert!(names.contains(&"C"), "C should be at depth 2");
        assert!(!names.contains(&"D"), "D should not appear at depth <= 2");
    }

    #[test]
    fn blast_radius_full_chain() {
        let db = Database::open_in_memory().unwrap();
        setup_graph(&db);

        let result = blast_radius(&db, "A", 10).unwrap().expect("symbol not found");
        let names: Vec<&str> = result.affected.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"B"));
        assert!(names.contains(&"C"));
        assert!(names.contains(&"D"));
        assert_eq!(result.total_affected, 3);
    }

    #[test]
    fn blast_radius_returns_none_for_unknown_symbol() {
        let db = Database::open_in_memory().unwrap();
        let result = blast_radius(&db, "DoesNotExist", 3).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn blast_radius_symbol_with_no_callers() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('b.cs', 'h', 'csharp', 0)",
            [],
        ).unwrap();
        let fid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'Lonely', 'NS.Lonely', 'class', 1, 0)",
            [fid],
        ).unwrap();

        let result = blast_radius(&db, "Lonely", 5).unwrap().expect("symbol should exist");
        assert_eq!(result.center.name, "Lonely");
        assert!(result.affected.is_empty());
        assert_eq!(result.total_affected, 0);
    }
}
