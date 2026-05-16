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
use serde::{Deserialize, Serialize};

use super::hierarchy_drill::{files_level, symbols_level};
use super::hierarchy_workspace::{directories_level, packages_level, services_level};

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
// Breadcrumb helpers
// ---------------------------------------------------------------------------

pub(super) fn workspace_breadcrumb(current_level: &str) -> Vec<Breadcrumb> {
    vec![Breadcrumb {
        label: "Workspace".to_string(),
        level: current_level.to_string(),
        scope: None,
    }]
}


pub(super) fn files_breadcrumbs(scope: Option<&str>) -> Vec<Breadcrumb> {
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

pub(super) fn symbols_breadcrumbs(scope: Option<&str>, package: Option<&str>) -> Vec<Breadcrumb> {
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

pub(super) trait OptionalExt<T> {
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
#[path = "hierarchy_tests.rs"]
mod tests;
