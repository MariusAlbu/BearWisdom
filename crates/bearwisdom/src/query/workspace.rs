// =============================================================================
// query/workspace.rs  —  workspace / monorepo level queries
//
// Answers the question "how is this multi-package repo structured?" by
// aggregating per-package statistics and cross-package edge counts.
//
// All queries gracefully handle single-project repos where the `packages`
// table is empty — they return empty vecs or zero counts rather than errors.
// =============================================================================

use crate::db::Database;
use crate::query::QueryResult;
use crate::query::architecture::HotspotSymbol;
use anyhow::Context;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Aggregated statistics for a single detected package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageStats {
    pub name: String,
    pub path: String,
    pub kind: Option<String>,
    pub file_count: u32,
    pub symbol_count: u32,
    pub edge_count: u32,
}

/// Workspace-level overview: per-package breakdown plus cross-package coupling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceOverview {
    pub packages: Vec<PackageStats>,
    /// Total number of edges whose source and target live in different packages.
    pub total_cross_package_edges: u32,
    /// Symbols referenced by 2 or more distinct packages — shared hotspots.
    pub shared_hotspots: Vec<HotspotSymbol>,
}

/// A directed dependency between two packages inferred from the edge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageDependency {
    pub source_package: String,
    pub target_package: String,
    pub edge_count: u32,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// List all detected packages with their per-package file/symbol/edge counts.
///
/// Returns an empty vec for single-project repos (empty `packages` table).
pub fn list_packages(db: &Database) -> QueryResult<Vec<PackageStats>> {
    let _timer = db.timer("workspace_list_packages");
    let conn = db.conn();

    // Use scalar subqueries to avoid fan-out from multi-table LEFT JOINs.
    // The edges table has no `id` column (composite PK), so we count via
    // a subquery that aggregates over symbols belonging to this package.
    let mut stmt = conn
        .prepare_cached(
            "SELECT p.name,
                    p.path,
                    p.kind,
                    (SELECT COUNT(*) FROM files f WHERE f.package_id = p.id)
                        AS file_count,
                    (SELECT COUNT(*) FROM symbols s
                     JOIN files f ON f.id = s.file_id
                     WHERE f.package_id = p.id)
                        AS symbol_count,
                    (SELECT COUNT(*) FROM edges e
                     JOIN symbols s ON s.id = e.source_id
                     JOIN files   f ON f.id = s.file_id
                     WHERE f.package_id = p.id)
                        AS edge_count
             FROM packages p
             ORDER BY file_count DESC, p.name",
        )
        .context("Failed to prepare list_packages query")?;

    let rows = stmt
        .query_map([], |row| {
            Ok(PackageStats {
                name:         row.get(0)?,
                path:         row.get(1)?,
                kind:         row.get(2)?,
                file_count:   row.get::<_, u32>(3).unwrap_or(0),
                symbol_count: row.get::<_, u32>(4).unwrap_or(0),
                edge_count:   row.get::<_, u32>(5).unwrap_or(0),
            })
        })
        .context("Failed to execute list_packages query")?;

    let packages = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect list_packages rows")?;

    Ok(packages)
}

/// Workspace-level overview: per-package stats, cross-package edge count,
/// and shared hotspot symbols (referenced by 2+ distinct packages).
///
/// Returns a `WorkspaceOverview` with empty/zero fields for single-project
/// repos — the caller does not need to handle the absent-packages case.
pub fn workspace_overview(db: &Database) -> QueryResult<WorkspaceOverview> {
    let _timer = db.timer("workspace_overview");

    let packages = list_packages(db)?;

    // If the packages table is empty this is a single-project repo.
    // Return a zero/empty overview rather than running the heavier queries.
    if packages.is_empty() {
        return Ok(WorkspaceOverview {
            packages,
            total_cross_package_edges: 0,
            shared_hotspots: vec![],
        });
    }

    let conn = db.conn();

    // --- Cross-package edge count ---
    let total_cross_package_edges: u32 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM edges e
             JOIN symbols s1 ON e.source_id = s1.id
             JOIN files   f1 ON s1.file_id  = f1.id
             JOIN symbols s2 ON e.target_id = s2.id
             JOIN files   f2 ON s2.file_id  = f2.id
             WHERE f1.package_id IS NOT NULL
               AND f2.package_id IS NOT NULL
               AND f1.package_id != f2.package_id",
            [],
            |r| r.get(0),
        )
        .context("Failed to count cross-package edges")?;

    // --- Shared hotspots: symbols referenced by 2+ distinct packages ---
    // A symbol is a "shared hotspot" when callers live in at least two
    // different packages.  We use the symbol's own package (via its file)
    // to label it and order by distinct-caller-package count descending.
    let mut hotspot_stmt = conn
        .prepare_cached(
            "SELECT s.name,
                    s.qualified_name,
                    s.kind,
                    f_tgt.path                       AS file_path,
                    COUNT(DISTINCT f_src.package_id) AS caller_package_count
             FROM edges e
             JOIN symbols s    ON s.id             = e.target_id
             JOIN files   f_tgt ON f_tgt.id        = s.file_id
             JOIN symbols s_src ON s_src.id        = e.source_id
             JOIN files   f_src ON f_src.id        = s_src.file_id
             WHERE f_src.package_id IS NOT NULL
               AND f_tgt.package_id IS NOT NULL
               AND f_src.package_id != f_tgt.package_id
             GROUP BY s.id, s.name, s.qualified_name, s.kind, f_tgt.path
             HAVING COUNT(DISTINCT f_src.package_id) >= 2
             ORDER BY caller_package_count DESC
             LIMIT 20",
        )
        .context("Failed to prepare shared hotspots query")?;

    let hotspot_rows = hotspot_stmt
        .query_map([], |row| {
            Ok(HotspotSymbol {
                name:          row.get(0)?,
                qualified_name: row.get(1)?,
                kind:          row.get(2)?,
                file_path:     row.get(3)?,
                incoming_refs: row.get(4)?,
            })
        })
        .context("Failed to execute shared hotspots query")?;

    let shared_hotspots = hotspot_rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect shared hotspot rows")?;

    Ok(WorkspaceOverview {
        packages,
        total_cross_package_edges,
        shared_hotspots,
    })
}

/// Which packages depend on which, inferred from cross-package edges.
///
/// An edge from a symbol in package A to a symbol in package B means A
/// depends on B.  Rows are ordered by edge count descending.
///
/// Returns an empty vec for single-project repos.
pub fn package_dependencies(db: &Database) -> QueryResult<Vec<PackageDependency>> {
    let _timer = db.timer("package_dependencies");
    let conn = db.conn();

    let mut stmt = conn
        .prepare_cached(
            "SELECT p_src.name,
                    p_tgt.name,
                    COUNT(*)          AS edge_count
             FROM edges e
             JOIN symbols s1  ON e.source_id   = s1.id
             JOIN files   f1  ON s1.file_id    = f1.id
             JOIN packages p_src ON f1.package_id = p_src.id
             JOIN symbols s2  ON e.target_id   = s2.id
             JOIN files   f2  ON s2.file_id    = f2.id
             JOIN packages p_tgt ON f2.package_id = p_tgt.id
             WHERE p_src.id != p_tgt.id
             GROUP BY p_src.name, p_tgt.name
             ORDER BY edge_count DESC",
        )
        .context("Failed to prepare package_dependencies query")?;

    let rows = stmt
        .query_map([], |row| {
            Ok(PackageDependency {
                source_package: row.get(0)?,
                target_package: row.get(1)?,
                edge_count:     row.get(2)?,
            })
        })
        .context("Failed to execute package_dependencies query")?;

    let deps = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect package_dependency rows")?;

    Ok(deps)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn setup_two_packages(db: &Database) {
        let conn = db.conn();

        // Two packages
        conn.execute(
            "INSERT INTO packages (name, path, kind) VALUES ('pkg-a', 'packages/a', 'cargo')",
            [],
        )
        .unwrap();
        let pkg_a = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO packages (name, path, kind) VALUES ('pkg-b', 'packages/b', 'cargo')",
            [],
        )
        .unwrap();
        let pkg_b = conn.last_insert_rowid();

        // One file per package
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed, package_id) VALUES ('packages/a/lib.rs', 'h1', 'rust', 0, ?1)",
            rusqlite::params![pkg_a],
        )
        .unwrap();
        let file_a = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed, package_id) VALUES ('packages/b/lib.rs', 'h2', 'rust', 0, ?1)",
            rusqlite::params![pkg_b],
        )
        .unwrap();
        let file_b = conn.last_insert_rowid();

        // One symbol per file
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'fn_a', 'a::fn_a', 'function', 1, 0)",
            rusqlite::params![file_a],
        )
        .unwrap();
        let sym_a = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'fn_b', 'b::fn_b', 'function', 1, 0)",
            rusqlite::params![file_b],
        )
        .unwrap();
        let sym_b = conn.last_insert_rowid();

        // pkg-a calls pkg-b (cross-package edge)
        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, confidence) VALUES (?1, ?2, 'calls', 1.0)",
            rusqlite::params![sym_a, sym_b],
        )
        .unwrap();
    }

    #[test]
    fn list_packages_returns_both_packages() {
        let db = Database::open_in_memory().unwrap();
        setup_two_packages(&db);

        let pkgs = list_packages(&db).unwrap();
        assert_eq!(pkgs.len(), 2);

        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"pkg-a"));
        assert!(names.contains(&"pkg-b"));
    }

    #[test]
    fn list_packages_empty_for_single_project() {
        let db = Database::open_in_memory().unwrap();
        let pkgs = list_packages(&db).unwrap();
        assert!(pkgs.is_empty());
    }

    #[test]
    fn workspace_overview_counts_cross_package_edge() {
        let db = Database::open_in_memory().unwrap();
        setup_two_packages(&db);

        let overview = workspace_overview(&db).unwrap();
        assert_eq!(overview.total_cross_package_edges, 1);
        assert_eq!(overview.packages.len(), 2);
    }

    #[test]
    fn workspace_overview_empty_for_single_project() {
        let db = Database::open_in_memory().unwrap();
        let overview = workspace_overview(&db).unwrap();
        assert_eq!(overview.total_cross_package_edges, 0);
        assert!(overview.packages.is_empty());
        assert!(overview.shared_hotspots.is_empty());
    }

    #[test]
    fn package_dependencies_detects_direction() {
        let db = Database::open_in_memory().unwrap();
        setup_two_packages(&db);

        let deps = package_dependencies(&db).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].source_package, "pkg-a");
        assert_eq!(deps[0].target_package, "pkg-b");
        assert_eq!(deps[0].edge_count, 1);
    }

    #[test]
    fn package_dependencies_empty_for_single_project() {
        let db = Database::open_in_memory().unwrap();
        let deps = package_dependencies(&db).unwrap();
        assert!(deps.is_empty());
    }
}
