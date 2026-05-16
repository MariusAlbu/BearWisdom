// =============================================================================
// query/hierarchy_drill.rs  —  scoped drill-down zoom levels
//
// Two level builders that take a scope (package path or file path) and return
// the next zoom layer beneath it:
//
//   files_level    — files inside a given package + file-to-file aggregate edges
//   symbols_level  — symbols inside a given file + direct symbol edges
// =============================================================================

use crate::db::Database;
use crate::query::QueryResult;
use anyhow::Context;
use std::collections::HashMap;

use super::hierarchy::{
    HierarchyEdge, HierarchyNode, HierarchyResult, OptionalExt,
    files_breadcrumbs, symbols_breadcrumbs,
};

// ---------------------------------------------------------------------------
// Level: files (scoped to a package)
// ---------------------------------------------------------------------------

pub(super) fn files_level(db: &Database, scope: Option<&str>, cap: usize) -> QueryResult<HierarchyResult> {
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

pub(super) fn symbols_level(db: &Database, scope: Option<&str>, cap: usize) -> QueryResult<HierarchyResult> {
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
