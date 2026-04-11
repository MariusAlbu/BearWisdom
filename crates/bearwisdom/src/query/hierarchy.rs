// =============================================================================
// query/hierarchy.rs  —  hierarchical graph query (four zoom levels)
//
// Returns a graph (nodes + edges) at one of four drill-down levels:
//
//   services  — service packages (or all packages) + service/k8s flow edges
//               plus aggregated cross-package code edges
//   packages  — all packages + cross-package edge counts
//   files     — files in a specific package + file-to-file edge aggregation
//   symbols   — symbols in a specific file + direct edges
//
// Breadcrumbs track the navigation path so UIs can render a back-button trail.
//
// All queries handle single-project repos gracefully (empty packages table):
//   services  → empty nodes/edges
//   packages  → empty nodes/edges
//   files     → returns all files (falls back when scope is absent)
//   symbols   → works normally against the files table
// =============================================================================

use crate::db::Database;
use crate::query::QueryResult;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public result types
// ---------------------------------------------------------------------------

/// A node at any zoom level (service, package, file, or symbol).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchyNode {
    /// Stable ID for edge referencing.
    /// Format: "pkg:<path>", "file:<path>", or "<symbol_id>" (integer string).
    pub id: String,
    /// Short display name.
    pub name: String,
    /// "service", "package", "file", "class", "method", etc.
    pub kind: String,
    /// Populated at file and symbol levels.
    pub file_path: Option<String>,
    /// Package path this node belongs to.
    pub package: Option<String>,
    /// Weight signal: symbol_count for packages, edge_count for files,
    /// incoming_edge_count for symbols.
    pub weight: u32,
    /// Files in package, symbols in file, etc.
    pub child_count: u32,
    /// JSON blob: ports+build_context for services; language for files.
    pub metadata: Option<String>,
}

/// A directed edge between two [`HierarchyNode`]s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchyEdge {
    /// Matches `HierarchyNode.id`.
    pub source: String,
    pub target: String,
    /// "service_dependency", "cross_package", "file_dependency", "calls", etc.
    pub kind: String,
    /// Count of underlying edges aggregated into this one.
    pub weight: u32,
    pub confidence: f64,
}

/// The full result returned by [`hierarchical_graph`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchyResult {
    pub nodes: Vec<HierarchyNode>,
    pub edges: Vec<HierarchyEdge>,
    /// "services", "packages", "files", or "symbols".
    pub level: String,
    /// Package path or file path that scopes this view.
    pub scope: Option<String>,
    /// Navigation breadcrumbs from workspace root to current view.
    pub breadcrumbs: Vec<Breadcrumb>,
}

/// One step in the navigation trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breadcrumb {
    pub label: String,
    pub level: String,
    pub scope: Option<String>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Return the hierarchical graph at the requested zoom level.
///
/// * `level`     — "services", "packages", "files", or "symbols"
/// * `scope`     — required for "files" (package path) and "symbols" (file path);
///                 ignored for "services" and "packages"
/// * `max_nodes` — hard cap; 0 defaults to 500
pub fn hierarchical_graph(
    db: &Database,
    level: &str,
    scope: Option<&str>,
    max_nodes: usize,
) -> QueryResult<HierarchyResult> {
    let _timer = db.timer("hierarchical_graph");
    let cap = if max_nodes == 0 { 500 } else { max_nodes.min(5_000) };

    // Strip node-ID prefixes that the frontend sends as scope values.
    // Node IDs use "pkg:<path>" and "file:<path>" format, but backend
    // queries expect bare paths.
    let scope = scope.map(|s| {
        s.strip_prefix("pkg:")
            .or_else(|| s.strip_prefix("file:"))
            .or_else(|| s.strip_prefix("dir:"))
            .unwrap_or(s)
    });

    match level {
        "services" => {
            let result = services_level(db, cap)?;
            if result.nodes.is_empty() {
                // No packages at all — show directory groups as pseudo-packages.
                directories_level(db, cap)
            } else {
                Ok(result)
            }
        }
        "packages" => {
            let result = packages_level(db, cap)?;
            if result.nodes.is_empty() {
                directories_level(db, cap)
            } else {
                Ok(result)
            }
        }
        "files"    => files_level(db, scope, cap),
        "symbols"  => symbols_level(db, scope, cap),
        other => Err(anyhow::anyhow!(
            "Unknown hierarchy level '{other}'. Expected: services, packages, files, symbols"
        )
        .into()),
    }
}

// ---------------------------------------------------------------------------
// Level: services
// ---------------------------------------------------------------------------

fn services_level(db: &Database, cap: usize) -> QueryResult<HierarchyResult> {
    let conn = db.conn();

    // Determine whether any packages are flagged as services.
    let service_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM packages WHERE is_service = 1",
            [],
            |r| r.get(0),
        )
        .context("Failed to count service packages")?;

    let where_clause = if service_count > 0 {
        "WHERE p.is_service = 1"
    } else {
        ""
    };

    // Nodes: service packages (or all packages when none are flagged).
    let sql = format!(
        "SELECT p.id, p.name, p.path, p.kind, p.is_service,
                (SELECT COUNT(*) FROM files f WHERE f.package_id = p.id AND f.origin = 'internal') AS file_count,
                (SELECT COUNT(*) FROM symbols s
                 JOIN files f ON s.file_id = f.id
                 WHERE f.package_id = p.id AND s.origin = 'internal') AS symbol_count
         FROM packages p
         {where_clause}
         ORDER BY symbol_count DESC
         LIMIT {cap}"
    );

    let mut stmt = conn.prepare(&sql).context("Failed to prepare services node query")?;

    let rows = stmt
        .query_map([], |row| {
            let pkg_id: i64       = row.get(0)?;
            let name: String      = row.get(1)?;
            let path: String      = row.get(2)?;
            let kind: Option<String> = row.get(3)?;
            let is_service: i64   = row.get(4)?;
            let file_count: u32   = row.get::<_, u32>(5).unwrap_or(0);
            let symbol_count: u32 = row.get::<_, u32>(6).unwrap_or(0);
            Ok((pkg_id, name, path, kind, is_service, file_count, symbol_count))
        })
        .context("Failed to execute services node query")?;

    let mut nodes: Vec<HierarchyNode> = Vec::new();
    let mut pkg_path_to_node_id: HashMap<String, String> = HashMap::new();

    for row in rows {
        let (_, name, path, kind, is_service, file_count, symbol_count) =
            row.context("Failed to read services row")?;
        let node_id = format!("pkg:{path}");
        let node_kind = if is_service == 1 {
            "service".to_string()
        } else {
            kind.unwrap_or_else(|| "package".to_string())
        };
        pkg_path_to_node_id.insert(path.clone(), node_id.clone());
        nodes.push(HierarchyNode {
            id: node_id,
            name,
            kind: node_kind,
            file_path: None,
            package: Some(path),
            weight: symbol_count,
            child_count: file_count,
            metadata: None,
        });
    }

    if nodes.is_empty() {
        return Ok(HierarchyResult {
            nodes,
            edges: vec![],
            level: "services".to_string(),
            scope: None,
            breadcrumbs: workspace_breadcrumb("services"),
        });
    }

    // Edges: service/k8s flow edges mapped to package paths.
    let mut edges: Vec<HierarchyEdge> = Vec::new();
    let mut edge_map: HashMap<(String, String, String), (u32, f64)> = HashMap::new();

    {
        let mut stmt = conn
            .prepare_cached(
                "SELECT p_src.path, p_tgt.path, fe.edge_type, COUNT(*) AS cnt,
                        AVG(fe.confidence) AS avg_conf
                 FROM flow_edges fe
                 JOIN files f_src ON fe.source_file_id = f_src.id
                 JOIN packages p_src ON f_src.package_id = p_src.id
                 JOIN files f_tgt ON fe.target_file_id = f_tgt.id
                 JOIN packages p_tgt ON f_tgt.package_id = p_tgt.id
                 WHERE fe.edge_type IN ('service_dependency', 'k8s_service_reference')
                   AND p_src.id != p_tgt.id
                 GROUP BY p_src.path, p_tgt.path, fe.edge_type",
            )
            .context("Failed to prepare service flow edge query")?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, f64>(4)?,
                ))
            })
            .context("Failed to execute service flow edge query")?;

        for row in rows {
            let (src_path, tgt_path, edge_type, cnt, conf) =
                row.context("Failed to read flow edge row")?;
            // Only keep edges between nodes we are showing.
            if pkg_path_to_node_id.contains_key(&src_path)
                && pkg_path_to_node_id.contains_key(&tgt_path)
            {
                let key = (src_path, tgt_path, edge_type);
                let entry = edge_map.entry(key).or_insert((0, 0.0));
                entry.0 += cnt;
                entry.1 = conf; // last writer wins; avg over flow edges is close enough
            }
        }
    }

    // Also add aggregated cross-package code edges (same as packages level).
    {
        let mut stmt = conn
            .prepare_cached(
                "SELECT p_src.path, p_tgt.path, COUNT(*) AS cnt
                 FROM edges e
                 JOIN symbols s1 ON e.source_id = s1.id
                 JOIN files f1 ON s1.file_id = f1.id
                 JOIN packages p_src ON f1.package_id = p_src.id
                 JOIN symbols s2 ON e.target_id = s2.id
                 JOIN files f2 ON s2.file_id = f2.id
                 JOIN packages p_tgt ON f2.package_id = p_tgt.id
                 WHERE p_src.id != p_tgt.id
                 GROUP BY p_src.path, p_tgt.path",
            )
            .context("Failed to prepare service cross-package edge query")?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                ))
            })
            .context("Failed to execute service cross-package edge query")?;

        for row in rows {
            let (src_path, tgt_path, cnt) = row.context("Failed to read cross-package row")?;
            if pkg_path_to_node_id.contains_key(&src_path)
                && pkg_path_to_node_id.contains_key(&tgt_path)
            {
                let key = (src_path, tgt_path, "cross_package".to_string());
                let entry = edge_map.entry(key).or_insert((0, 0.8));
                entry.0 += cnt;
            }
        }
    }

    for ((src_path, tgt_path, kind), (weight, confidence)) in edge_map {
        edges.push(HierarchyEdge {
            source: format!("pkg:{src_path}"),
            target: format!("pkg:{tgt_path}"),
            kind,
            weight,
            confidence,
        });
    }

    Ok(HierarchyResult {
        nodes,
        edges,
        level: "services".to_string(),
        scope: None,
        breadcrumbs: workspace_breadcrumb("services"),
    })
}

// ---------------------------------------------------------------------------
// Level: packages
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Level: directories (fallback for repos with no packages)
// ---------------------------------------------------------------------------

/// Groups files by their top-level directory (first path segment) and returns
/// those directories as drillable nodes.  Used as a fallback when the packages
/// table is empty so the graph shows a useful high-level view instead of
/// hundreds of individual file nodes.
fn directories_level(db: &Database, cap: usize) -> QueryResult<HierarchyResult> {
    let conn = db.conn();

    // Group files by top-level directory.
    let mut stmt = conn.prepare(
        "SELECT f.path,
                (SELECT COUNT(*) FROM symbols s WHERE s.file_id = f.id AND s.origin = 'internal') AS sym_count,
                f.language
         FROM files f
         WHERE f.origin = 'internal'"
    ).context("Failed to prepare directory scan")?;

    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, u32>(1).unwrap_or(0),
            r.get::<_, String>(2)?,
        ))
    }).context("Failed to execute directory scan")?;

    // Accumulate per-directory stats.
    let mut dirs: HashMap<String, (u32, u32, HashMap<String, u32>)> = HashMap::new(); // dir → (file_count, symbol_count, {lang → count})

    for row in rows {
        let (path, sym_count, lang) = row.context("Failed to read file row")?;
        // Extract top-level directory (e.g., "server" from "server/src/main.ts").
        // Files at root go into a "(root)" bucket.
        let dir = path.split('/').next()
            .filter(|seg| path.contains('/'))
            .unwrap_or("(root)")
            .to_string();

        let entry = dirs.entry(dir).or_insert_with(|| (0, 0, HashMap::new()));
        entry.0 += 1;
        entry.1 += sym_count;
        *entry.2.entry(lang).or_insert(0) += 1;
    }

    // Sort by symbol count descending, cap.
    let mut dir_list: Vec<(String, u32, u32, HashMap<String, u32>)> = dirs
        .into_iter()
        .map(|(dir, (fc, sc, langs))| (dir, fc, sc, langs))
        .collect();
    dir_list.sort_by(|a, b| b.2.cmp(&a.2));
    dir_list.truncate(cap);

    let mut nodes = Vec::new();
    for (dir, file_count, symbol_count, langs) in &dir_list {
        let primary_lang = langs.iter().max_by_key(|(_, c)| *c).map(|(l, _)| l.as_str()).unwrap_or("unknown");
        let metadata = serde_json::json!({ "language": primary_lang }).to_string();
        nodes.push(HierarchyNode {
            id: format!("dir:{dir}"),
            name: dir.clone(),
            kind: "package".to_string(), // render as package shape
            file_path: None,
            package: Some(dir.clone()),
            weight: *symbol_count,
            child_count: *file_count,
            metadata: Some(metadata),
        });
    }

    // Cross-directory edges (aggregate symbol edges by directory).
    let dir_set: std::collections::HashSet<&str> = dir_list.iter().map(|(d, _, _, _)| d.as_str()).collect();
    let mut edge_map: HashMap<(String, String), u32> = HashMap::new();

    let mut edge_stmt = conn.prepare(
        "SELECT f1.path, f2.path
         FROM edges e
         JOIN symbols s1 ON e.source_id = s1.id
         JOIN files f1 ON s1.file_id = f1.id
         JOIN symbols s2 ON e.target_id = s2.id
         JOIN files f2 ON s2.file_id = f2.id"
    ).context("Failed to prepare directory edge query")?;

    let edge_rows = edge_stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    }).context("Failed to execute directory edge query")?;

    for row in edge_rows {
        let (src_path, tgt_path) = row.context("Failed to read edge row")?;
        let src_dir = src_path.split('/').next()
            .filter(|_| src_path.contains('/'))
            .unwrap_or("(root)");
        let tgt_dir = tgt_path.split('/').next()
            .filter(|_| tgt_path.contains('/'))
            .unwrap_or("(root)");

        if src_dir == tgt_dir { continue; }
        if !dir_set.contains(src_dir) || !dir_set.contains(tgt_dir) { continue; }

        *edge_map.entry((src_dir.to_string(), tgt_dir.to_string())).or_insert(0) += 1;
    }

    let edges: Vec<HierarchyEdge> = edge_map
        .into_iter()
        .map(|((src, tgt), weight)| HierarchyEdge {
            source: format!("dir:{src}"),
            target: format!("dir:{tgt}"),
            kind: "cross_directory".to_string(),
            weight,
            confidence: 0.8,
        })
        .collect();

    Ok(HierarchyResult {
        nodes,
        edges,
        level: "packages".to_string(), // report as packages level so drill-down goes to files
        scope: None,
        breadcrumbs: vec![Breadcrumb {
            label: "Workspace".to_string(),
            level: "packages".to_string(),
            scope: None,
        }],
    })
}

// ---------------------------------------------------------------------------
// Level: packages
// ---------------------------------------------------------------------------

fn packages_level(db: &Database, cap: usize) -> QueryResult<HierarchyResult> {
    let conn = db.conn();

    let sql = format!(
        "SELECT p.id, p.name, p.path, p.kind, p.is_service,
                (SELECT COUNT(*) FROM files f WHERE f.package_id = p.id) AS file_count,
                (SELECT COUNT(*) FROM symbols s
                 JOIN files f ON s.file_id = f.id
                 WHERE f.package_id = p.id) AS symbol_count
         FROM packages p
         ORDER BY symbol_count DESC
         LIMIT {cap}"
    );

    let mut stmt = conn.prepare(&sql).context("Failed to prepare packages node query")?;

    let rows = stmt
        .query_map([], |row| {
            let name: String         = row.get(1)?;
            let path: String         = row.get(2)?;
            let kind: Option<String> = row.get(3)?;
            let is_service: i64      = row.get(4)?;
            let file_count: u32      = row.get::<_, u32>(5).unwrap_or(0);
            let symbol_count: u32    = row.get::<_, u32>(6).unwrap_or(0);
            Ok((name, path, kind, is_service, file_count, symbol_count))
        })
        .context("Failed to execute packages node query")?;

    let mut nodes: Vec<HierarchyNode> = Vec::new();
    let mut pkg_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    for row in rows {
        let (name, path, kind, is_service, file_count, symbol_count) =
            row.context("Failed to read packages row")?;
        let node_id = format!("pkg:{path}");
        let node_kind = if is_service == 1 {
            "service".to_string()
        } else {
            kind.unwrap_or_else(|| "package".to_string())
        };
        pkg_paths.insert(path.clone());
        nodes.push(HierarchyNode {
            id: node_id,
            name,
            kind: node_kind,
            file_path: None,
            package: Some(path),
            weight: symbol_count,
            child_count: file_count,
            metadata: None,
        });
    }

    if nodes.is_empty() {
        return Ok(HierarchyResult {
            nodes,
            edges: vec![],
            level: "packages".to_string(),
            scope: None,
            breadcrumbs: workspace_breadcrumb("packages"),
        });
    }

    // Edges: cross-package symbol edge aggregation.
    let mut stmt = conn
        .prepare_cached(
            "SELECT p_src.path, p_tgt.path, COUNT(*) AS edge_count
             FROM edges e
             JOIN symbols s1 ON e.source_id = s1.id
             JOIN files f1 ON s1.file_id = f1.id
             JOIN packages p_src ON f1.package_id = p_src.id
             JOIN symbols s2 ON e.target_id = s2.id
             JOIN files f2 ON s2.file_id = f2.id
             JOIN packages p_tgt ON f2.package_id = p_tgt.id
             WHERE p_src.id != p_tgt.id
             GROUP BY p_src.path, p_tgt.path
             ORDER BY edge_count DESC",
        )
        .context("Failed to prepare package cross-edge query")?;

    let edge_rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u32>(2)?,
            ))
        })
        .context("Failed to execute package cross-edge query")?;

    let mut edges: Vec<HierarchyEdge> = Vec::new();
    for row in edge_rows {
        let (src_path, tgt_path, edge_count) = row.context("Failed to read package edge row")?;
        if pkg_paths.contains(&src_path) && pkg_paths.contains(&tgt_path) {
            edges.push(HierarchyEdge {
                source: format!("pkg:{src_path}"),
                target: format!("pkg:{tgt_path}"),
                kind: "cross_package".to_string(),
                weight: edge_count,
                confidence: 0.8,
            });
        }
    }

    Ok(HierarchyResult {
        nodes,
        edges,
        level: "packages".to_string(),
        scope: None,
        breadcrumbs: workspace_breadcrumb("packages"),
    })
}

// ---------------------------------------------------------------------------
// Level: files (scoped to a package)
// ---------------------------------------------------------------------------

fn files_level(db: &Database, scope: Option<&str>, cap: usize) -> QueryResult<HierarchyResult> {
    let conn = db.conn();

    // Resolve the package_id from the scope path.
    // If scope is absent or the package is not found, fall back to all files
    // (handles single-project repos and mis-specified scopes gracefully).
    let package_id: Option<i64> = match scope {
        Some(pkg_path) => conn
            .query_row(
                "SELECT id FROM packages WHERE path = ?1",
                rusqlite::params![pkg_path],
                |r| r.get(0),
            )
            .optional()
            .context("Failed to look up package by path")?,
        None => None,
    };

    // Nodes: files scoped by package, directory prefix, or all files.
    enum ScopeKind { Package(i64), DirPrefix(String), All }

    let scope_kind = if let Some(pkg_id) = package_id {
        ScopeKind::Package(pkg_id)
    } else if let Some(s) = scope {
        // No matching package — treat scope as a directory prefix.
        let prefix = if s.ends_with('/') { s.to_string() } else { format!("{s}/") };
        ScopeKind::DirPrefix(prefix)
    } else {
        ScopeKind::All
    };

    let node_sql = match &scope_kind {
        ScopeKind::Package(_) => format!(
            "SELECT f.id, f.path, f.language, f.package_id,
                    (SELECT COUNT(*) FROM symbols s WHERE s.file_id = f.id AND s.origin = 'internal') AS symbol_count
             FROM files f
             WHERE f.package_id = ?1 AND f.origin = 'internal'
             ORDER BY symbol_count DESC
             LIMIT {cap}"
        ),
        ScopeKind::DirPrefix(_) => format!(
            "SELECT f.id, f.path, f.language, f.package_id,
                    (SELECT COUNT(*) FROM symbols s WHERE s.file_id = f.id AND s.origin = 'internal') AS symbol_count
             FROM files f
             WHERE f.path LIKE (?1 || '%') AND f.origin = 'internal'
             ORDER BY symbol_count DESC
             LIMIT {cap}"
        ),
        ScopeKind::All => format!(
            "SELECT f.id, f.path, f.language, f.package_id,
                    (SELECT COUNT(*) FROM symbols s WHERE s.file_id = f.id AND s.origin = 'internal') AS symbol_count
             FROM files f
             WHERE f.origin = 'internal'
             ORDER BY symbol_count DESC
             LIMIT {cap}"
        ),
    };

    let mut stmt = conn.prepare(&node_sql).context("Failed to prepare files node query")?;

    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(i64, String, String, Option<i64>, u32)> {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get::<_, u32>(4).unwrap_or(0),
        ))
    };

    let file_rows = match &scope_kind {
        ScopeKind::Package(pkg_id) => stmt.query_map(rusqlite::params![*pkg_id], map_row),
        ScopeKind::DirPrefix(prefix) => stmt.query_map(rusqlite::params![prefix], map_row),
        ScopeKind::All => stmt.query_map([], map_row),
    }
    .context("Failed to execute files node query")?;

    let mut nodes: Vec<HierarchyNode> = Vec::new();
    let mut file_path_to_id: HashMap<String, i64> = HashMap::new();
    let mut file_id_to_path: HashMap<i64, String> = HashMap::new();

    // Resolve package path for labelling (may be None for single-project).
    let package_path: Option<String> = match (scope, package_id) {
        (Some(p), _) => Some(p.to_string()),
        _ => None,
    };

    for row in file_rows {
        let (file_id, path, language, _pkg_id, symbol_count) =
            row.context("Failed to read files row")?;

        // File name = last path component.
        let name = path
            .rsplit('/')
            .next()
            .unwrap_or(&path)
            .to_string();

        let node_id = format!("file:{path}");
        file_path_to_id.insert(path.clone(), file_id);
        file_id_to_path.insert(file_id, path.clone());

        nodes.push(HierarchyNode {
            id: node_id,
            name,
            kind: "file".to_string(),
            file_path: Some(path.clone()),
            package: package_path.clone(),
            weight: symbol_count,
            child_count: symbol_count,
            metadata: Some(format!(r#"{{"language":"{language}"}}"#)),
        });
    }

    if nodes.is_empty() {
        let breadcrumbs = files_breadcrumbs(scope);
        return Ok(HierarchyResult {
            nodes,
            edges: vec![],
            level: "files".to_string(),
            scope: scope.map(str::to_string),
            breadcrumbs,
        });
    }

    // Edges: file-to-file edge aggregation.
    // When scoped to a package, only include edges where the source file is in
    // the package.  The target may be in any package (cross-package links are
    // still useful to show).
    let edge_sql = match &scope_kind {
        ScopeKind::Package(_) =>
            "SELECT f_src.path, f_tgt.path, e.kind, COUNT(*) AS edge_count, AVG(e.confidence) AS avg_conf
             FROM edges e
             JOIN symbols s1 ON e.source_id = s1.id
             JOIN files f_src ON s1.file_id = f_src.id
             JOIN symbols s2 ON e.target_id = s2.id
             JOIN files f_tgt ON s2.file_id = f_tgt.id
             WHERE f_src.package_id = ?1
               AND f_src.id != f_tgt.id
             GROUP BY f_src.path, f_tgt.path, e.kind
             ORDER BY edge_count DESC",
        ScopeKind::DirPrefix(_) =>
            "SELECT f_src.path, f_tgt.path, e.kind, COUNT(*) AS edge_count, AVG(e.confidence) AS avg_conf
             FROM edges e
             JOIN symbols s1 ON e.source_id = s1.id
             JOIN files f_src ON s1.file_id = f_src.id
             JOIN symbols s2 ON e.target_id = s2.id
             JOIN files f_tgt ON s2.file_id = f_tgt.id
             WHERE f_src.path LIKE (?1 || '%')
               AND f_src.id != f_tgt.id
             GROUP BY f_src.path, f_tgt.path, e.kind
             ORDER BY edge_count DESC",
        ScopeKind::All =>
            "SELECT f_src.path, f_tgt.path, e.kind, COUNT(*) AS edge_count, AVG(e.confidence) AS avg_conf
             FROM edges e
             JOIN symbols s1 ON e.source_id = s1.id
             JOIN files f_src ON s1.file_id = f_src.id
             JOIN symbols s2 ON e.target_id = s2.id
             JOIN files f_tgt ON s2.file_id = f_tgt.id
             WHERE f_src.id != f_tgt.id
             GROUP BY f_src.path, f_tgt.path, e.kind
             ORDER BY edge_count DESC",
    };

    let mut estmt = conn.prepare_cached(edge_sql).context("Failed to prepare files edge query")?;

    let emap_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(String, String, String, u32, f64)> {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get::<_, u32>(3)?,
            row.get::<_, f64>(4)?,
        ))
    };

    let edge_rows = match &scope_kind {
        ScopeKind::Package(pkg_id) => estmt.query_map(rusqlite::params![*pkg_id], emap_row),
        ScopeKind::DirPrefix(prefix) => estmt.query_map(rusqlite::params![prefix], emap_row),
        ScopeKind::All => estmt.query_map([], emap_row),
    }
    .context("Failed to execute files edge query")?;

    // Aggregate edges by (src_file, tgt_file) — collapse edge kinds into "file_dependency"
    // to keep the graph manageable; individual kind breakdown is available at the symbols level.
    let mut agg_edges: HashMap<(String, String), (u32, f64)> = HashMap::new();
    for row in edge_rows {
        let (src_path, tgt_path, _kind, cnt, conf) = row.context("Failed to read files edge row")?;
        // Only include edges where at least the source node is in our visible set.
        if file_path_to_id.contains_key(&src_path) {
            let key = (src_path, tgt_path);
            let entry = agg_edges.entry(key).or_insert((0, conf));
            entry.0 += cnt;
        }
    }

    let edges: Vec<HierarchyEdge> = agg_edges
        .into_iter()
        .map(|((src_path, tgt_path), (weight, confidence))| HierarchyEdge {
            source: format!("file:{src_path}"),
            target: format!("file:{tgt_path}"),
            kind: "file_dependency".to_string(),
            weight,
            confidence,
        })
        .collect();

    let breadcrumbs = files_breadcrumbs(scope);
    Ok(HierarchyResult {
        nodes,
        edges,
        level: "files".to_string(),
        scope: scope.map(str::to_string),
        breadcrumbs,
    })
}

// ---------------------------------------------------------------------------
// Level: symbols (scoped to a file)
// ---------------------------------------------------------------------------

fn symbols_level(db: &Database, scope: Option<&str>, cap: usize) -> QueryResult<HierarchyResult> {
    let conn = db.conn();

    let file_path = scope.unwrap_or("");

    // Nodes: symbols in the file.
    let node_sql = format!(
        "SELECT s.id, s.name, s.qualified_name, s.kind, f.path,
                (SELECT COUNT(*) FROM packages p
                 JOIN files ff ON ff.package_id = p.id
                 WHERE ff.id = s.file_id LIMIT 1) AS _unused,
                s.incoming_edge_count
         FROM symbols s
         JOIN files f ON s.file_id = f.id
         WHERE f.path = ?1
         ORDER BY s.incoming_edge_count DESC, s.line
         LIMIT {cap}"
    );

    let mut stmt = conn.prepare(&node_sql).context("Failed to prepare symbols node query")?;

    let rows = stmt
        .query_map(rusqlite::params![file_path], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, u32>(6)?,
            ))
        })
        .context("Failed to execute symbols node query")?;

    let mut nodes: Vec<HierarchyNode> = Vec::new();
    let mut symbol_ids: Vec<i64> = Vec::new();

    // Resolve package path from the file's package (if any).
    let package_for_file: Option<String> = conn
        .query_row(
            "SELECT p.path FROM packages p
             JOIN files f ON f.package_id = p.id
             WHERE f.path = ?1
             LIMIT 1",
            rusqlite::params![file_path],
            |r| r.get(0),
        )
        .optional()
        .context("Failed to look up package for file")?;

    for row in rows {
        let (sym_id, name, _qname, kind, fpath, incoming) =
            row.context("Failed to read symbols row")?;
        symbol_ids.push(sym_id);
        nodes.push(HierarchyNode {
            id: sym_id.to_string(),
            name,
            kind,
            file_path: Some(fpath),
            package: package_for_file.clone(),
            weight: incoming,
            child_count: 0,
            metadata: None,
        });
    }

    if nodes.is_empty() {
        let breadcrumbs = symbols_breadcrumbs(scope, package_for_file.as_deref());
        return Ok(HierarchyResult {
            nodes,
            edges: vec![],
            level: "symbols".to_string(),
            scope: scope.map(str::to_string),
            breadcrumbs,
        });
    }

    // Edges: direct edges where source OR target is in our symbol set.
    // We use a temp table so we get both directions in a single index-friendly JOIN.
    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS _hier_syms (id INTEGER PRIMARY KEY)",
    )
    .context("Failed to create hierarchy temp table")?;
    conn.execute("DELETE FROM _hier_syms", [])
        .context("Failed to clear hierarchy temp table")?;

    {
        let tx = conn
            .unchecked_transaction()
            .context("Failed to begin hierarchy temp transaction")?;
        let mut ins = tx.prepare_cached("INSERT OR IGNORE INTO _hier_syms (id) VALUES (?1)")?;
        for &id in &symbol_ids {
            ins.execute([id])?;
        }
        drop(ins);
        tx.commit().context("Failed to commit hierarchy temp inserts")?;
    }

    let mut estmt = conn
        .prepare(
            "SELECT e.source_id, e.target_id, e.kind, e.confidence
             FROM edges e
             WHERE e.source_id IN (SELECT id FROM _hier_syms)
                OR e.target_id IN (SELECT id FROM _hier_syms)",
        )
        .context("Failed to prepare symbols edge query")?;

    let edge_rows = estmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
            ))
        })
        .context("Failed to execute symbols edge query")?;

    let mut edges: Vec<HierarchyEdge> = Vec::new();
    for row in edge_rows {
        let (src_id, tgt_id, kind, confidence) = row.context("Failed to read symbols edge row")?;
        edges.push(HierarchyEdge {
            source: src_id.to_string(),
            target: tgt_id.to_string(),
            kind,
            weight: 1,
            confidence,
        });
    }

    // Clean up temp table.
    let _ = conn.execute("DELETE FROM _hier_syms", []);

    let breadcrumbs = symbols_breadcrumbs(scope, package_for_file.as_deref());
    Ok(HierarchyResult {
        nodes,
        edges,
        level: "symbols".to_string(),
        scope: scope.map(str::to_string),
        breadcrumbs,
    })
}

// ---------------------------------------------------------------------------
// Breadcrumb helpers
// ---------------------------------------------------------------------------

fn workspace_breadcrumb(current_level: &str) -> Vec<Breadcrumb> {
    vec![Breadcrumb {
        label: "Workspace".to_string(),
        level: current_level.to_string(),
        scope: None,
    }]
}

fn files_breadcrumbs(scope: Option<&str>) -> Vec<Breadcrumb> {
    let mut crumbs = vec![
        Breadcrumb {
            label: "Workspace".to_string(),
            level: "packages".to_string(),
            scope: None,
        },
    ];
    if let Some(pkg_path) = scope {
        // Last segment of the package path as label.
        let label = pkg_path.rsplit('/').next().unwrap_or(pkg_path).to_string();
        crumbs.push(Breadcrumb {
            label,
            level: "files".to_string(),
            scope: Some(pkg_path.to_string()),
        });
    }
    crumbs
}

fn symbols_breadcrumbs(scope: Option<&str>, package: Option<&str>) -> Vec<Breadcrumb> {
    let mut crumbs = vec![
        Breadcrumb {
            label: "Workspace".to_string(),
            level: "packages".to_string(),
            scope: None,
        },
    ];
    if let Some(pkg_path) = package {
        let label = pkg_path.rsplit('/').next().unwrap_or(pkg_path).to_string();
        crumbs.push(Breadcrumb {
            label,
            level: "files".to_string(),
            scope: Some(pkg_path.to_string()),
        });
    }
    if let Some(file_path) = scope {
        let label = file_path.rsplit('/').next().unwrap_or(file_path).to_string();
        crumbs.push(Breadcrumb {
            label,
            level: "symbols".to_string(),
            scope: Some(file_path.to_string()),
        });
    }
    crumbs
}

// ---------------------------------------------------------------------------
// Extension trait for optional query_row
// ---------------------------------------------------------------------------

trait OptionalExt<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn setup_two_package_repo(db: &Database) -> (i64, i64, i64, i64, i64, i64) {
        let conn = db.conn();

        conn.execute(
            "INSERT INTO packages (name, path, kind, is_service) VALUES ('pkg-a', 'packages/a', 'cargo', 0)",
            [],
        ).unwrap();
        let pkg_a = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO packages (name, path, kind, is_service) VALUES ('pkg-b', 'packages/b', 'cargo', 0)",
            [],
        ).unwrap();
        let pkg_b = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed, package_id) VALUES ('packages/a/lib.rs', 'h1', 'rust', 0, ?1)",
            rusqlite::params![pkg_a],
        ).unwrap();
        let file_a = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed, package_id) VALUES ('packages/b/lib.rs', 'h2', 'rust', 0, ?1)",
            rusqlite::params![pkg_b],
        ).unwrap();
        let file_b = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'fn_a', 'a::fn_a', 'function', 1, 0)",
            rusqlite::params![file_a],
        ).unwrap();
        let sym_a = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'fn_b', 'b::fn_b', 'function', 1, 0)",
            rusqlite::params![file_b],
        ).unwrap();
        let sym_b = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)",
            rusqlite::params![sym_a, sym_b],
        ).unwrap();

        (pkg_a, pkg_b, file_a, file_b, sym_a, sym_b)
    }

    // ---- packages level ----

    #[test]
    fn packages_level_returns_nodes_and_edge() {
        let db = Database::open_in_memory().unwrap();
        setup_two_package_repo(&db);

        let result = hierarchical_graph(&db, "packages", None, 500).unwrap();
        assert_eq!(result.level, "packages");
        assert_eq!(result.nodes.len(), 2);
        assert_eq!(result.edges.len(), 1);
        assert_eq!(result.edges[0].kind, "cross_package");
        assert_eq!(result.edges[0].weight, 1);
    }

    #[test]
    fn packages_level_empty_for_single_project() {
        let db = Database::open_in_memory().unwrap();
        // No packages inserted.
        let result = hierarchical_graph(&db, "packages", None, 500).unwrap();
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    // ---- services level ----

    #[test]
    fn services_level_falls_back_to_all_packages_when_none_flagged() {
        let db = Database::open_in_memory().unwrap();
        setup_two_package_repo(&db);

        let result = hierarchical_graph(&db, "services", None, 500).unwrap();
        // No is_service=1 packages, so it falls back to all packages.
        assert_eq!(result.nodes.len(), 2);
        assert_eq!(result.level, "services");
    }

    #[test]
    fn services_level_only_shows_service_packages_when_flagged() {
        let db = Database::open_in_memory().unwrap();
        setup_two_package_repo(&db);
        // Flag pkg-a as a service.
        db.conn()
            .execute(
                "UPDATE packages SET is_service = 1 WHERE path = 'packages/a'",
                [],
            )
            .unwrap();

        let result = hierarchical_graph(&db, "services", None, 500).unwrap();
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].kind, "service");
    }

    // ---- files level ----

    #[test]
    fn files_level_scoped_to_package() {
        let db = Database::open_in_memory().unwrap();
        setup_two_package_repo(&db);

        let result = hierarchical_graph(&db, "files", Some("packages/a"), 500).unwrap();
        assert_eq!(result.level, "files");
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].file_path.as_deref(), Some("packages/a/lib.rs"));
        // One cross-package file edge exists: packages/a/lib.rs → packages/b/lib.rs.
        // The files level shows outbound edges even when the target file is in another package.
        assert_eq!(result.edges.len(), 1);
        assert_eq!(result.edges[0].kind, "file_dependency");
        assert_eq!(result.edges[0].source, "file:packages/a/lib.rs");
        assert_eq!(result.edges[0].target, "file:packages/b/lib.rs");
    }

    #[test]
    fn files_level_no_scope_returns_all_files() {
        let db = Database::open_in_memory().unwrap();
        setup_two_package_repo(&db);

        let result = hierarchical_graph(&db, "files", None, 500).unwrap();
        assert_eq!(result.nodes.len(), 2);
        // Cross-file edge exists: packages/a/lib.rs → packages/b/lib.rs.
        assert_eq!(result.edges.len(), 1);
    }

    // ---- symbols level ----

    #[test]
    fn symbols_level_returns_symbols_in_file() {
        let db = Database::open_in_memory().unwrap();
        setup_two_package_repo(&db);

        let result = hierarchical_graph(&db, "symbols", Some("packages/a/lib.rs"), 500).unwrap();
        assert_eq!(result.level, "symbols");
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].name, "fn_a");
        // Edge exists: fn_a calls fn_b (fn_b is not in this file but the edge is included).
        assert_eq!(result.edges.len(), 1);
        assert_eq!(result.edges[0].kind, "calls");
    }

    #[test]
    fn symbols_level_empty_file_returns_empty() {
        let db = Database::open_in_memory().unwrap();
        let result = hierarchical_graph(&db, "symbols", Some("nonexistent.rs"), 500).unwrap();
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    // ---- breadcrumbs ----

    #[test]
    fn breadcrumbs_always_start_with_workspace() {
        let db = Database::open_in_memory().unwrap();
        for level in ["services", "packages", "files", "symbols"] {
            let result = hierarchical_graph(&db, level, None, 500).unwrap();
            assert_eq!(result.breadcrumbs[0].label, "Workspace");
        }
    }

    #[test]
    fn files_breadcrumbs_include_package() {
        let db = Database::open_in_memory().unwrap();
        setup_two_package_repo(&db);
        let result = hierarchical_graph(&db, "files", Some("packages/a"), 500).unwrap();
        assert_eq!(result.breadcrumbs.len(), 2);
        assert_eq!(result.breadcrumbs[1].label, "a");
        assert_eq!(result.breadcrumbs[1].level, "files");
    }

    // ---- error handling ----

    #[test]
    fn unknown_level_returns_error() {
        let db = Database::open_in_memory().unwrap();
        let result = hierarchical_graph(&db, "bogus", None, 500);
        assert!(result.is_err());
    }

    // ---- node id format ----

    #[test]
    fn package_node_ids_have_pkg_prefix() {
        let db = Database::open_in_memory().unwrap();
        setup_two_package_repo(&db);
        let result = hierarchical_graph(&db, "packages", None, 500).unwrap();
        for node in &result.nodes {
            assert!(node.id.starts_with("pkg:"), "unexpected id: {}", node.id);
        }
    }

    #[test]
    fn file_node_ids_have_file_prefix() {
        let db = Database::open_in_memory().unwrap();
        setup_two_package_repo(&db);
        let result = hierarchical_graph(&db, "files", None, 500).unwrap();
        for node in &result.nodes {
            assert!(node.id.starts_with("file:"), "unexpected id: {}", node.id);
        }
    }
}
