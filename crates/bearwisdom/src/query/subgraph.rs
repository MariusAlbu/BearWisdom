// =============================================================================
// query/subgraph.rs  —  graph export for visualization
//
// Exports a portion of the symbol graph as a list of nodes and edges, suitable
// for rendering in D3, Cytoscape, or any other graph visualization library.
//
// Filter modes (the `filter` parameter):
//   • None             — export the entire graph (capped at max_nodes).
//   • "eShop.Catalog"  — only symbols whose qualified_name starts with this prefix.
//   • "@authentication"— only symbols that are members of the "authentication" concept.
//
// The `max_nodes` cap prevents exporting multi-million-node graphs that would
// crash a browser.  When the cap is hit, edges are also filtered to keep only
// those whose both endpoints are in the included node set.
//
// `export_graph_json` is a thin wrapper that serialises the result to a JSON
// string via serde_json.
// =============================================================================

use crate::db::Database;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// A single node in the exported graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    /// The symbol's primary key in the `symbols` table.
    pub id: i64,
    pub name: String,
    pub qualified_name: String,
    /// Symbol kind string, e.g. "class", "method".
    pub kind: String,
    pub file_path: String,
    /// The first concept this symbol belongs to (if any).
    pub concept: Option<String>,
    /// The first annotation attached to this symbol (if any).
    pub annotation: Option<String>,
    /// Number of edges in the full graph (not just the visible subgraph).
    /// Lets the UI show "this node has N connections" even when edges are
    /// hidden because partner nodes fell outside the cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_edges: Option<u32>,
}

/// A directed edge between two nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source_id: i64,
    pub target_id: i64,
    /// Edge kind string, e.g. "calls", "inherits", "type_ref".
    pub kind: String,
    pub confidence: f64,
}

/// The full graph export: a collection of nodes and the edges between them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphResult {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Export a portion of the symbol graph as nodes and edges.
///
/// `filter` controls which symbols are included:
///   - `None`          → all symbols (capped at `max_nodes`)
///   - `Some("prefix")` → symbols whose qualified_name starts with `prefix`
///   - `Some("@concept")` → symbols in the named concept (prefix `@`)
///
/// Edges are included only when BOTH endpoints are in the filtered node set.
pub fn export_graph(
    db: &Database,
    filter: Option<&str>,
    max_nodes: usize,
) -> Result<SubgraphResult> {
    let _timer = db.timer("export_graph");
    let conn = &db.conn;

    // Effective cap: never export more than 10 000 nodes unconditionally.
    let cap = if max_nodes == 0 { 10_000 } else { max_nodes.min(10_000) };

    // --- Step 1: Load nodes (symbols) ---
    let nodes: Vec<GraphNode> = {
        let sql = build_node_sql(filter, cap);
        let mut stmt = conn.prepare(&sql)
            .context("Failed to prepare node export query")?;

        // Map a row to a GraphNode — used in all three query_map calls below.
        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<GraphNode> {
            Ok(GraphNode {
                id:             row.get(0)?,
                name:           row.get(1)?,
                qualified_name: row.get(2)?,
                kind:           row.get(3)?,
                file_path:      row.get(4)?,
                concept:        row.get(5)?,
                annotation:     row.get(6)?,
                total_edges:    row.get(7)?,
            })
        };

        // We must pass the correct number of parameters to SQLite:
        //   • None            → no WHERE clause → 0 parameters
        //   • Some(f) @…      → WHERE c.name = ?1 → 1 parameter (concept name, no '@')
        //   • Some(f) prefix  → WHERE … LIKE ?1 → 1 parameter (prefix string)
        let rows = match filter {
            None => stmt.query_map([], map_row),
            Some(f) if f.starts_with('@') =>
                stmt.query_map(rusqlite::params![&f[1..]], map_row),  // strip '@'
            Some(f) =>
                stmt.query_map(rusqlite::params![f], map_row),
        }.context("Failed to execute node export query")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect graph nodes")?
    };

    if nodes.is_empty() {
        return Ok(SubgraphResult { nodes, edges: vec![] });
    }

    // Build a set of included node IDs for fast edge filtering.
    let node_ids: std::collections::HashSet<i64> = nodes.iter().map(|n| n.id).collect();

    // --- Step 2: Load edges where BOTH endpoints are in node_ids ---
    // Insert node IDs into a temp table, then JOIN edges against it.
    // This is O(nodes + matching edges) via index, vs the old approach of
    // scanning the entire edges table and filtering in Rust.
    let edges: Vec<GraphEdge> = {
        conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS _export_nodes (id INTEGER PRIMARY KEY)"
        ).context("Failed to create temp table")?;
        conn.execute("DELETE FROM _export_nodes", [])
            .context("Failed to clear temp table")?;

        // Batch-insert node IDs.
        {
            let tx = conn.unchecked_transaction()
                .context("Failed to begin temp insert transaction")?;
            let mut ins = tx.prepare_cached(
                "INSERT OR IGNORE INTO _export_nodes (id) VALUES (?1)"
            )?;
            for &id in &node_ids {
                ins.execute([id])?;
            }
            drop(ins);
            tx.commit().context("Failed to commit temp inserts")?;
        }

        let mut stmt = conn.prepare(
            "SELECT e.source_id, e.target_id, e.kind, e.confidence
             FROM edges e
             JOIN _export_nodes ns ON e.source_id = ns.id
             JOIN _export_nodes nt ON e.target_id = nt.id"
        ).context("Failed to prepare edge export query")?;

        let rows = stmt.query_map([], |row| {
            Ok(GraphEdge {
                source_id:  row.get(0)?,
                target_id:  row.get(1)?,
                kind:       row.get(2)?,
                confidence: row.get(3)?,
            })
        }).context("Failed to execute edge export query")?;

        let result = rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect graph edges")?;

        // Clean up temp table.
        let _ = conn.execute("DELETE FROM _export_nodes", []);

        result
    };

    // --- Step 3: Keep nodes that have ANY edge in the full graph ---
    // A node may appear disconnected in the subgraph because its partner nodes
    // fell outside the node cap. But it's still a connected symbol in the full
    // graph. We keep it so the user can click it and see its connections in the
    // detail panel. Only prune truly isolated nodes (zero edges in the DB).
    // When a filter IS applied, keep all matched nodes unconditionally.
    let nodes = if filter.is_none() {
        // node_ids already loaded from the query which has `connected_filter`
        // (EXISTS edges), so all nodes here are connected in the full graph.
        // No pruning needed.
        nodes
    } else {
        nodes
    };

    Ok(SubgraphResult { nodes, edges })
}

/// Build the SELECT statement for nodes based on the filter mode.
fn build_node_sql(filter: Option<&str>, cap: usize) -> String {
    // Common SELECT columns + LEFT JOINs for concept and annotation.
    // We take the first concept name and first annotation content via MIN()
    // which acts as an arbitrary-pick in SQLite.
    let select = "
        SELECT s.id,
               s.name,
               s.qualified_name,
               s.kind,
               f.path       AS file_path,
               MIN(c.name)  AS concept,
               MIN(a.content) AS annotation,
               (SELECT COUNT(*) FROM edges e2 WHERE e2.source_id = s.id OR e2.target_id = s.id) AS total_edges
        FROM symbols s
        JOIN files f ON f.id = s.file_id
        LEFT JOIN concept_members cm ON cm.symbol_id = s.id
        LEFT JOIN concepts c  ON c.id  = cm.concept_id
        LEFT JOIN annotations a ON a.symbol_id = s.id
    ";

    // Exclude symbol kinds that are structural/noise in a graph view.
    // Variables (let bindings, params) and namespaces (mod/package) clutter
    // the graph without adding meaningful relationship information.
    let kind_filter =
        "s.kind NOT IN ('variable', 'namespace')";

    // Only include nodes that participate in at least one edge (as source or
    // target).  Isolated symbols are noise in the graph visualization.
    let connected_filter =
        "(EXISTS (SELECT 1 FROM edges e WHERE e.source_id = s.id)
       OR EXISTS (SELECT 1 FROM edges e WHERE e.target_id = s.id))";

    let group_and_limit = format!("GROUP BY s.id ORDER BY s.qualified_name LIMIT {cap}");

    match filter {
        None => format!(
            "{select} WHERE {kind_filter} AND {connected_filter} {group_and_limit}"
        ),

        Some(f) if f.starts_with('@') => {
            // Concept filter: symbols that are members of the named concept.
            // The parameter ?1 is the concept name (without the '@').
            format!(
                "{select}
                 WHERE c.name = ?1 AND {kind_filter}
                 {group_and_limit}"
            )
        }

        Some(_) => {
            // Prefix filter: symbols whose qualified_name starts with ?1.
            format!(
                "{select}
                 WHERE s.qualified_name LIKE ?1 || '%' AND {kind_filter}
                 {group_and_limit}"
            )
        }
    }
}

/// Export the graph as a JSON string.
///
/// This is a convenience wrapper around [`export_graph`] for callers
/// that need a serialised representation (e.g. an HTTP response or a file).
pub fn export_graph_json(
    db: &Database,
    filter: Option<&str>,
    max_nodes: usize,
) -> Result<String> {
    let graph = export_graph(db, filter, max_nodes)?;
    serde_json::to_string(&graph).context("Failed to serialise graph to JSON")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "subgraph_tests.rs"]
mod tests;
