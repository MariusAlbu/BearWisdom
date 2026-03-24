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
    // We do this in Rust rather than with a complex SQL sub-query, because
    // the node set is already in memory and the SQL alternative (IN clause)
    // can be very slow for large ID sets.
    let edges: Vec<GraphEdge> = {
        let mut stmt = conn.prepare(
            "SELECT source_id, target_id, kind, confidence FROM edges"
        ).context("Failed to prepare edge export query")?;

        let rows = stmt.query_map([], |row| {
            Ok(GraphEdge {
                source_id:  row.get(0)?,
                target_id:  row.get(1)?,
                kind:       row.get(2)?,
                confidence: row.get(3)?,
            })
        }).context("Failed to execute edge export query")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect graph edges")?
            .into_iter()
            .filter(|e| node_ids.contains(&e.source_id) && node_ids.contains(&e.target_id))
            .collect()
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
               MIN(a.content) AS annotation
        FROM symbols s
        JOIN files f ON f.id = s.file_id
        LEFT JOIN concept_members cm ON cm.symbol_id = s.id
        LEFT JOIN concepts c  ON c.id  = cm.concept_id
        LEFT JOIN annotations a ON a.symbol_id = s.id
    ";

    let group_and_limit = format!("GROUP BY s.id ORDER BY s.qualified_name LIMIT {cap}");

    match filter {
        None => format!("{select} {group_and_limit}"),

        Some(f) if f.starts_with('@') => {
            // Concept filter: symbols that are members of the named concept.
            // The parameter ?1 is the concept name (without the '@').
            format!(
                "{select}
                 WHERE c.name = ?1
                 {group_and_limit}"
            )
        }

        Some(_) => {
            // Prefix filter: symbols whose qualified_name starts with ?1.
            format!(
                "{select}
                 WHERE s.qualified_name LIKE ?1 || '%'
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
mod tests {
    use super::*;
    use crate::db::Database;

    fn insert_symbol(db: &Database, path: &str, name: &str, qname: &str) -> i64 {
        let conn = &db.conn;
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'csharp', 0)
             ON CONFLICT(path) DO NOTHING",
            [path],
        ).unwrap();
        let fid: i64 = conn.query_row("SELECT id FROM files WHERE path=?1", [path], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, ?2, ?3, 'class', 1, 0)",
            rusqlite::params![fid, name, qname],
        ).unwrap();
        conn.last_insert_rowid()
    }

    fn insert_edge(db: &Database, src: i64, tgt: i64) {
        db.conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)",
            rusqlite::params![src, tgt],
        ).unwrap();
    }

    #[test]
    fn export_full_graph_includes_all_symbols_and_edges() {
        let db = Database::open_in_memory().unwrap();
        let s1 = insert_symbol(&db, "a.cs", "Foo", "App.Foo");
        let s2 = insert_symbol(&db, "b.cs", "Bar", "App.Bar");
        insert_edge(&db, s1, s2);

        let graph = export_graph(&db, None, 1000).unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
    }

    #[test]
    fn export_with_prefix_filter_excludes_other_namespaces() {
        let db = Database::open_in_memory().unwrap();
        let s1 = insert_symbol(&db, "a.cs", "CatalogService", "App.Catalog.CatalogService");
        let s2 = insert_symbol(&db, "b.cs", "OrderService",   "App.Orders.OrderService");
        insert_edge(&db, s1, s2);

        let graph = export_graph(&db, Some("App.Catalog"), 1000).unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].name, "CatalogService");
        // The edge connects to OrderService which is excluded → no edges.
        assert_eq!(graph.edges.len(), 0);
    }

    #[test]
    fn export_with_concept_filter() {
        let db = Database::open_in_memory().unwrap();
        let s1 = insert_symbol(&db, "a.cs", "AuthService", "App.Auth.AuthService");
        let _s2 = insert_symbol(&db, "b.cs", "Other",       "App.Other.Other");

        // Create concept and assign s1 to it.
        db.conn.execute(
            "INSERT INTO concepts (name, auto_pattern, created_at) VALUES ('auth', 'App.Auth.*', 0)",
            [],
        ).unwrap();
        let cid: i64 = db.conn.last_insert_rowid();
        db.conn.execute(
            "INSERT INTO concept_members (concept_id, symbol_id, auto_assigned) VALUES (?1, ?2, 1)",
            rusqlite::params![cid, s1],
        ).unwrap();

        let graph = export_graph(&db, Some("@auth"), 1000).unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].name, "AuthService");
    }

    #[test]
    fn export_respects_max_nodes_cap() {
        let db = Database::open_in_memory().unwrap();
        for i in 0..20 {
            insert_symbol(&db, "a.cs", &format!("Sym{i}"), &format!("App.Sym{i}"));
        }

        let graph = export_graph(&db, None, 5).unwrap();
        assert_eq!(graph.nodes.len(), 5, "Should respect max_nodes cap");
    }

    #[test]
    fn export_edges_only_between_included_nodes() {
        let db = Database::open_in_memory().unwrap();
        let s1 = insert_symbol(&db, "a.cs", "A", "NS.A");
        let s2 = insert_symbol(&db, "b.cs", "B", "NS.B");
        let s3 = insert_symbol(&db, "c.cs", "C", "Other.C");
        insert_edge(&db, s1, s2);
        insert_edge(&db, s1, s3); // s3 will be excluded by prefix filter

        let graph = export_graph(&db, Some("NS"), 1000).unwrap();
        assert_eq!(graph.nodes.len(), 2);
        // Only the edge between s1 and s2 should be included.
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].source_id, s1);
        assert_eq!(graph.edges[0].target_id, s2);
    }

    #[test]
    fn export_graph_json_is_valid_json() {
        let db = Database::open_in_memory().unwrap();
        insert_symbol(&db, "a.cs", "Foo", "App.Foo");

        let json = export_graph_json(&db, None, 100).unwrap();
        // Verify it parses without error and contains the expected keys.
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["nodes"].is_array());
        assert!(parsed["edges"].is_array());
    }

    #[test]
    fn export_empty_graph() {
        let db = Database::open_in_memory().unwrap();
        let graph = export_graph(&db, None, 1000).unwrap();
        assert!(graph.nodes.is_empty());
        assert!(graph.edges.is_empty());
    }
}
