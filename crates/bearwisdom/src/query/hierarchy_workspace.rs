// =============================================================================
// query/hierarchy_workspace.rs  —  workspace-aggregate zoom levels
//
// Three level builders that operate on the whole workspace, returning
// package- or directory-scoped nodes and aggregate cross-package edges:
//
//   services_level     — service-flagged packages (or all packages as fallback)
//   directories_level  — top-level directory groups (fallback when no packages)
//   packages_level     — every package + cross-package edge counts
// =============================================================================

use crate::db::Database;
use crate::query::QueryResult;
use anyhow::Context;
use std::collections::HashMap;

use super::hierarchy::{Breadcrumb, HierarchyEdge, HierarchyNode, HierarchyResult, workspace_breadcrumb};

// ---------------------------------------------------------------------------
// Level: services
// ---------------------------------------------------------------------------

pub(super) fn services_level(db: &Database, cap: usize) -> QueryResult<HierarchyResult> {
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
pub(super) fn directories_level(db: &Database, cap: usize) -> QueryResult<HierarchyResult> {
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

pub(super) fn packages_level(db: &Database, cap: usize) -> QueryResult<HierarchyResult> {
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

