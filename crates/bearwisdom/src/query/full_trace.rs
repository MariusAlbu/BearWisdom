// =============================================================================
// query/full_trace.rs  —  End-to-end flow tracing across call graph + flow edges
//
// Combines the regular `edges` table (calls, type_ref, instantiates) with the
// `flow_edges` table (DI bindings, HTTP calls, event handlers) to produce full
// execution traces from entry points to leaf nodes.
//
// Algorithm:
//   1. Start from a set of entry-point symbols (or all architecture entry points).
//   2. Walk outgoing `edges` (calls/type_ref/instantiates) forward.
//   3. At each symbol, check if it's the source or target of a `flow_edge`.
//      If so, create a "jump" to the other side (interface → implementation,
//      HTTP client → controller, event publisher → handler).
//   4. Continue walking from the jump target.
//   5. Stop at leaf nodes (no outgoing edges) or max depth.
//
// The result is a forest of TraceNode trees, one per entry point.
// =============================================================================

use crate::query::QueryResult;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::db::Database;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// One node in a full trace tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceNode {
    /// Symbol name (simple).
    pub name: String,
    /// Fully qualified name.
    pub qualified_name: String,
    /// Symbol kind (class, method, interface, etc.).
    pub kind: String,
    /// File where this symbol is defined.
    pub file_path: String,
    /// Line number.
    pub line: u32,
    /// How this node was reached from its parent.
    pub edge_kind: String,
    /// Depth from the root of this trace (0 = entry point).
    pub depth: u32,
    /// Children: symbols this node calls/references.
    pub children: Vec<TraceNode>,
}

/// A complete trace starting from one entry point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceRoot {
    /// The entry point symbol.
    pub entry: TraceNode,
    /// Total number of nodes in this trace tree.
    pub node_count: u32,
}

/// Result of tracing all entry points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullTraceResult {
    /// One trace tree per entry point.
    pub traces: Vec<TraceRoot>,
    /// Total unique symbols visited across all traces.
    pub total_symbols: u32,
    /// Summary: how many flow-edge jumps occurred.
    pub flow_jumps: u32,
}

// ---------------------------------------------------------------------------
// Internal: symbol row from DB
// ---------------------------------------------------------------------------

struct SymRow {
    id: i64,
    name: String,
    qualified_name: String,
    kind: String,
    file_path: String,
    line: u32,
}

// ---------------------------------------------------------------------------
// Internal: flow-edge jump map
// ---------------------------------------------------------------------------

/// Preloaded flow_edges indexed by symbol name for quick lookup.
struct FlowJumpMap {
    /// source_symbol → [(target_symbol, edge_type)]
    by_source: HashMap<String, Vec<(String, String)>>,
    /// target_symbol → [(source_symbol, edge_type)]  (reverse for DI: interface→impl)
    by_target: HashMap<String, Vec<(String, String)>>,
}

impl FlowJumpMap {
    fn load(db: &Database) -> QueryResult<Self> {
        let _timer = db.timer("flow_jump_map_load");
        let conn = db.conn();
        let mut by_source: HashMap<String, Vec<(String, String)>> = HashMap::new();
        let mut by_target: HashMap<String, Vec<(String, String)>> = HashMap::new();

        let mut stmt = conn.prepare(
            "SELECT source_symbol, target_symbol, edge_type
             FROM flow_edges
             WHERE source_symbol IS NOT NULL
               AND target_symbol IS NOT NULL"
        )?;

        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let src: String = row.get(0)?;
            let tgt: String = row.get(1)?;
            let et: String = row.get(2)?;

            by_source.entry(src.clone()).or_default().push((tgt.clone(), et.clone()));
            by_target.entry(tgt).or_default().push((src, et));
        }

        Ok(Self { by_source, by_target })
    }

    /// Get jump targets for a symbol (check both directions).
    fn jumps_for(&self, symbol: &str) -> Vec<(String, String)> {
        let mut result = Vec::new();
        // Forward: this symbol is a source (e.g., HTTP client → API controller)
        if let Some(targets) = self.by_source.get(symbol) {
            result.extend(targets.iter().cloned());
        }
        // Reverse: this symbol is a target (e.g., interface → check if there's an implementation)
        // For DI: the flow_edge goes impl→interface, so if we're at the interface,
        // the source is the implementation we should jump to.
        if let Some(sources) = self.by_target.get(symbol) {
            result.extend(sources.iter().cloned());
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Trace full execution flows starting from a specific symbol.
///
/// Walks outgoing edges (calls/type_ref/instantiates) and follows flow_edge
/// jumps (DI, HTTP, events) to produce end-to-end traces.
pub fn trace_from_symbol(
    db: &Database,
    symbol_name: &str,
    max_depth: u32,
) -> QueryResult<FullTraceResult> {
    let _timer = db.timer("trace_from_symbol");
    let conn = db.conn();
    let jumps = FlowJumpMap::load(db)?;
    let mut visited_global = HashSet::new();
    let mut flow_jump_count = 0u32;

    // Resolve the starting symbol.
    let starts = resolve_symbol_rows(conn, symbol_name)?;
    if starts.is_empty() {
        return Ok(FullTraceResult {
            traces: vec![],
            total_symbols: 0,
            flow_jumps: 0,
        });
    }

    let mut traces = Vec::new();

    for start in &starts {
        let mut visited = HashSet::new();
        visited.insert(start.id);

        let entry = build_trace_node(
            conn,
            start,
            "entry_point",
            0,
            max_depth,
            &jumps,
            &mut visited,
            &mut flow_jump_count,
        )?;

        let node_count = count_nodes(&entry);
        visited_global.extend(visited);

        traces.push(TraceRoot { entry, node_count });
    }

    Ok(FullTraceResult {
        traces,
        total_symbols: visited_global.len() as u32,
        flow_jumps: flow_jump_count,
    })
}

/// Trace full execution flows starting from graph-structural roots.
///
/// Roots are detected by topology, not naming conventions:
///   1. Flow-edge targets (HTTP endpoints, event handlers, DI implementations)
///   2. Symbols with outgoing `calls` edges but zero incoming `calls` edges (true DAG roots)
///   3. Symbols with highest outgoing degree (most connected, likely entry points)
///
/// Results are sorted by trace size (most nodes first).
pub fn trace_from_entry_points(
    db: &Database,
    max_depth: u32,
    max_traces: usize,
) -> QueryResult<FullTraceResult> {
    let _timer = db.timer("trace_from_entry_points");
    let conn = db.conn();
    let jumps = FlowJumpMap::load(db)?;

    // Find structural roots: symbols that have outgoing calls/type_ref/instantiates
    // edges but few or no incoming calls edges. These are natural "top of the call tree".
    // Also include flow-edge targets (controllers hit by HTTP, handlers hit by events).
    let mut stmt = conn.prepare(
        "SELECT s.id, s.name, s.qualified_name, s.kind, f.path, s.line,
                (SELECT COUNT(*) FROM edges e WHERE e.source_id = s.id
                   AND e.kind IN ('calls', 'type_ref', 'instantiates')) AS out_degree,
                (SELECT COUNT(*) FROM edges e WHERE e.target_id = s.id
                   AND e.kind = 'calls') AS in_calls
         FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.kind IN ('class', 'struct', 'method', 'function')
           AND (SELECT COUNT(*) FROM edges e2 WHERE e2.source_id = s.id
                  AND e2.kind IN ('calls', 'type_ref', 'instantiates')) > 0
         ORDER BY
           -- Prioritize: zero incoming calls first, then by outgoing degree
           CASE WHEN in_calls = 0 THEN 0 ELSE 1 END,
           out_degree DESC
         LIMIT ?1"
    )?;

    let mut roots: Vec<SymRow> = Vec::new();
    {
        // Request more candidates than max_traces since some will produce empty traces
        let mut rows = stmt.query([max_traces as i64 * 4])?;
        while let Some(row) = rows.next()? {
            roots.push(SymRow {
                id: row.get(0)?,
                name: row.get(1)?,
                qualified_name: row.get(2)?,
                kind: row.get(3)?,
                file_path: row.get(4)?,
                line: row.get(5)?,
            });
        }
    }

    // Also add flow-edge targets as roots (HTTP endpoints, event handlers)
    {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT s.id, s.name, s.qualified_name, s.kind, tf.path, s.line
             FROM flow_edges fe
             JOIN files tf ON tf.id = fe.target_file_id
             JOIN symbols s ON s.file_id = tf.id AND s.name = fe.target_symbol
             WHERE fe.target_symbol IS NOT NULL
             LIMIT ?1"
        )?;
        let mut rows = stmt.query([max_traces as i64 * 2])?;
        while let Some(row) = rows.next()? {
            roots.push(SymRow {
                id: row.get(0)?,
                name: row.get(1)?,
                qualified_name: row.get(2)?,
                kind: row.get(3)?,
                file_path: row.get(4)?,
                line: row.get(5)?,
            });
        }
    }

    let mut visited_global = HashSet::new();
    let mut flow_jump_count = 0u32;
    let mut traces = Vec::new();

    for start in &roots {
        if traces.len() >= max_traces {
            break;
        }

        if visited_global.contains(&start.id) {
            continue;
        }

        let mut visited = HashSet::new();
        visited.insert(start.id);

        let entry = build_trace_node(
            conn,
            start,
            "entry_point",
            0,
            max_depth,
            &jumps,
            &mut visited,
            &mut flow_jump_count,
        )?;

        let node_count = count_nodes(&entry);
        if node_count <= 1 {
            // Skip entry points with no outgoing flow — not interesting.
            continue;
        }

        visited_global.extend(visited);
        traces.push(TraceRoot { entry, node_count });
    }

    Ok(FullTraceResult {
        traces,
        total_symbols: visited_global.len() as u32,
        flow_jumps: flow_jump_count,
    })
}

// ---------------------------------------------------------------------------
// Internal: recursive trace builder
// ---------------------------------------------------------------------------

fn build_trace_node(
    conn: &rusqlite::Connection,
    sym: &SymRow,
    edge_kind: &str,
    depth: u32,
    max_depth: u32,
    jumps: &FlowJumpMap,
    visited: &mut HashSet<i64>,
    flow_jump_count: &mut u32,
) -> QueryResult<TraceNode> {
    let mut children = Vec::new();

    if depth < max_depth {
        // 0. For container symbols (class, struct, interface), descend into
        //    child methods/properties and trace from them.  This is critical
        //    because entry points are typically classes, not methods.
        let is_container = matches!(
            sym.kind.as_str(),
            "class" | "struct" | "interface" | "namespace"
        );
        if is_container {
            let mut child_stmt = conn.prepare_cached(
                "SELECT s.id, s.name, s.qualified_name, s.kind, f.path, s.line
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE s.scope_path = ?1
                   AND s.kind IN ('method', 'function', 'constructor', 'property')
                 ORDER BY s.line
                 LIMIT 30"
            )?;

            let mut member_rows = Vec::new();
            {
                let mut rows = child_stmt.query([&sym.qualified_name])?;
                while let Some(row) = rows.next()? {
                    member_rows.push(SymRow {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        qualified_name: row.get(2)?,
                        kind: row.get(3)?,
                        file_path: row.get(4)?,
                        line: row.get(5)?,
                    });
                }
            }

            for member in &member_rows {
                if !visited.insert(member.id) {
                    continue;
                }
                // Only include members that have outgoing edges (skip empty methods).
                let has_outgoing: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM edges WHERE source_id = ?1 AND kind IN ('calls', 'type_ref', 'instantiates'))",
                    [member.id],
                    |r| r.get(0),
                ).unwrap_or(false);

                if !has_outgoing {
                    // Also check flow_edge jumps for this member.
                    let has_flow = !jumps.jumps_for(&member.qualified_name).is_empty()
                        || !jumps.jumps_for(&member.name).is_empty();
                    if !has_flow {
                        continue;
                    }
                }

                let child = build_trace_node(
                    conn, member, "member", depth + 1, max_depth, jumps, visited, flow_jump_count,
                )?;
                children.push(child);
            }
        }

        // 1. Follow regular edges (calls, type_ref, instantiates).
        let mut stmt = conn.prepare_cached(
            "SELECT s.id, s.name, s.qualified_name, s.kind, f.path, s.line, e.kind
             FROM edges e
             JOIN symbols s ON s.id = e.target_id
             JOIN files f ON f.id = s.file_id
             WHERE e.source_id = ?1
               AND e.kind IN ('calls', 'type_ref', 'instantiates')
             ORDER BY e.source_line
             LIMIT 20"
        )?;

        let mut call_rows = Vec::new();
        {
            let mut rows = stmt.query([sym.id])?;
            while let Some(row) = rows.next()? {
                call_rows.push((
                    SymRow {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        qualified_name: row.get(2)?,
                        kind: row.get(3)?,
                        file_path: row.get(4)?,
                        line: row.get(5)?,
                    },
                    row.get::<_, String>(6)?,
                ));
            }
        }

        for (child_sym, ek) in &call_rows {
            if !visited.insert(child_sym.id) {
                continue;
            }
            let child = build_trace_node(
                conn, child_sym, ek, depth + 1, max_depth, jumps, visited, flow_jump_count,
            )?;
            children.push(child);
        }

        // 2. Follow flow_edge jumps.
        let flow_targets = jumps.jumps_for(&sym.qualified_name);
        // Also check simple name (flow_edges often store just the short name).
        let mut all_targets = flow_targets;
        if sym.name != sym.qualified_name {
            all_targets.extend(jumps.jumps_for(&sym.name));
        }

        for (target_name, flow_type) in &all_targets {
            let target_rows = resolve_symbol_rows(conn, target_name)?;
            for target_sym in &target_rows {
                if !visited.insert(target_sym.id) {
                    continue;
                }
                *flow_jump_count += 1;
                let child = build_trace_node(
                    conn, target_sym, flow_type, depth + 1, max_depth, jumps, visited, flow_jump_count,
                )?;
                children.push(child);
            }
        }
    }

    Ok(TraceNode {
        name: sym.name.clone(),
        qualified_name: sym.qualified_name.clone(),
        kind: sym.kind.clone(),
        file_path: sym.file_path.clone(),
        line: sym.line,
        edge_kind: edge_kind.to_string(),
        depth,
        children,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_symbol_rows(conn: &rusqlite::Connection, name: &str) -> QueryResult<Vec<SymRow>> {
    // Try qualified name first, then simple name.
    let mut stmt = conn.prepare_cached(
        "SELECT s.id, s.name, s.qualified_name, s.kind, f.path, s.line
         FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.qualified_name = ?1
            OR s.name = ?1
         ORDER BY CASE WHEN s.qualified_name = ?1 THEN 0 ELSE 1 END
         LIMIT 5"
    ).context("Failed to prepare resolve_symbol_rows")?;

    let mut results = Vec::new();
    let mut rows = stmt.query([name])?;
    while let Some(row) = rows.next()? {
        results.push(SymRow {
            id: row.get(0)?,
            name: row.get(1)?,
            qualified_name: row.get(2)?,
            kind: row.get(3)?,
            file_path: row.get(4)?,
            line: row.get(5)?,
        });
    }
    Ok(results)
}

fn count_nodes(node: &TraceNode) -> u32 {
    1 + node.children.iter().map(count_nodes).sum::<u32>()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_empty_symbol() {
        let db = Database::open_in_memory().unwrap();
        let result = trace_from_symbol(&db, "nonexistent", 5).unwrap();
        assert!(result.traces.is_empty());
        assert_eq!(result.total_symbols, 0);
    }

    #[test]
    fn test_trace_basic() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        // Create file.
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/api.rs', 'h', 'rust', 0)",
            [],
        ).unwrap();
        let fid = conn.last_insert_rowid();

        // Create symbols: Controller → Service → Repository
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'GetItems', 'Api.GetItems', 'method', 10, 0)",
            [fid],
        ).unwrap();
        let ctrl_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'ItemService', 'Svc.ItemService', 'class', 20, 0)",
            [fid],
        ).unwrap();
        let svc_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'ItemRepo', 'Repo.ItemRepo', 'class', 30, 0)",
            [fid],
        ).unwrap();
        let repo_id = conn.last_insert_rowid();

        // Edges: GetItems → ItemService → ItemRepo
        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 0.9)",
            rusqlite::params![ctrl_id, svc_id],
        ).unwrap();
        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 0.9)",
            rusqlite::params![svc_id, repo_id],
        ).unwrap();

        let result = trace_from_symbol(&db, "Api.GetItems", 5).unwrap();
        assert_eq!(result.traces.len(), 1);
        let root = &result.traces[0];
        assert_eq!(root.entry.name, "GetItems");
        assert_eq!(root.entry.children.len(), 1);
        assert_eq!(root.entry.children[0].name, "ItemService");
        assert_eq!(root.entry.children[0].children.len(), 1);
        assert_eq!(root.entry.children[0].children[0].name, "ItemRepo");
        assert_eq!(root.node_count, 3);
    }

    #[test]
    fn test_trace_with_flow_jump() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/app.rs', 'h', 'rust', 0)",
            [],
        ).unwrap();
        let fid = conn.last_insert_rowid();

        // HttpClient → IService (interface) -[DI jump]→ ServiceImpl → Repo
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'HttpClient', 'Http.Client', 'class', 1, 0)",
            [fid],
        ).unwrap();
        let client_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'IService', 'App.IService', 'interface', 10, 0)",
            [fid],
        ).unwrap();
        let iface_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'ServiceImpl', 'App.ServiceImpl', 'class', 20, 0)",
            [fid],
        ).unwrap();
        let impl_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'Repo', 'Data.Repo', 'class', 30, 0)",
            [fid],
        ).unwrap();
        let repo_id = conn.last_insert_rowid();

        // Regular edge: Client → IService
        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'type_ref', 0.8)",
            rusqlite::params![client_id, iface_id],
        ).unwrap();

        // Regular edge: ServiceImpl → Repo
        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 0.9)",
            rusqlite::params![impl_id, repo_id],
        ).unwrap();

        // Flow edge: DI binding ServiceImpl → IService
        conn.execute(
            "INSERT INTO flow_edges (source_file_id, source_symbol, target_file_id, target_symbol, edge_type, confidence)
             VALUES (?1, 'App.ServiceImpl', ?1, 'App.IService', 'di_binding', 1.0)",
            [fid],
        ).unwrap();

        let result = trace_from_symbol(&db, "Http.Client", 5).unwrap();
        assert_eq!(result.traces.len(), 1);

        let root = &result.traces[0];
        assert_eq!(root.entry.name, "HttpClient");

        // Client → IService (type_ref) → ServiceImpl (di_binding jump) → Repo (calls)
        assert!(root.node_count >= 3, "Expected at least 3 nodes, got {}", root.node_count);
        assert!(result.flow_jumps >= 1, "Expected at least 1 flow jump, got {}", result.flow_jumps);
    }
}
