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

/// One row per directed (source_pkg, target_pkg) pair in the workspace graph.
///
/// The three signal families are kept separate so downstream consumers can
/// weight them as they see fit: `code` counts symbol-level references, `flow`
/// counts cross-tier wiring (HTTP calls, DB entity links), and `declared_dep`
/// surfaces the manifest-level intent captured in `package_deps`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceGraphEdge {
    pub source_package: String,
    pub target_package: String,
    /// Symbol-level code edges (calls, type_ref, inherits, implements,
    /// instantiates, imports, lsp_resolved).
    pub code_edges: u32,
    /// Per-kind breakdown of `code_edges`. Kind strings come from
    /// `EdgeKind::as_str` — e.g. "calls", "type_ref", "inherits".
    /// Only kinds with count > 0 appear.
    pub code_by_kind: Vec<(String, u32)>,
    /// Cross-tier flow edges (http_call, db_entity).
    pub flow_edges: u32,
    pub flow_by_kind: Vec<(String, u32)>,
    /// True when the source package's manifest declares the target package
    /// (either by `declared_name` or by folder-derived `name`).
    pub declared_dep: bool,
    /// Total = code_edges + flow_edges. A convenience field for sorting.
    pub total_edges: u32,
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

/// Aggregated workspace graph: one row per (source_pkg, target_pkg) pair
/// with per-kind breakdown and a declared-dependency flag.
///
/// Merges three signal sources:
///   - code edges from `edges` (split by `EdgeKind` into code vs flow)
///   - manifest-declared dependencies from `package_deps`
///
/// Rows with zero code/flow edges still appear when the source package's
/// manifest declares the target package — that way "I depend on it but
/// never actually call into it" is still visible.
///
/// Returns an empty vec for single-project repos.
pub fn workspace_graph(db: &Database) -> QueryResult<Vec<WorkspaceGraphEdge>> {
    use std::collections::HashMap;

    let _timer = db.timer("workspace_graph");
    let conn = db.conn();

    // Classify an EdgeKind string as code vs flow. Keep in sync with EdgeKind.
    fn is_flow(kind: &str) -> bool {
        matches!(kind, "http_call" | "db_entity")
    }

    // --- Code & flow edges, grouped by (src_pkg, tgt_pkg, kind) ---
    //
    // Symbols without a package (root configs, shared scripts) are excluded
    // — they'd collapse to a bogus self-edge and add noise. Self-edges
    // (src == tgt) are skipped for the same reason.
    let mut edge_stmt = conn
        .prepare_cached(
            "SELECT p_src.name,
                    p_tgt.name,
                    e.kind,
                    COUNT(*) AS cnt
             FROM edges e
             JOIN symbols  s1 ON s1.id = e.source_id
             JOIN files    f1 ON f1.id = s1.file_id
             JOIN packages p_src ON p_src.id = f1.package_id
             JOIN symbols  s2 ON s2.id = e.target_id
             JOIN files    f2 ON f2.id = s2.file_id
             JOIN packages p_tgt ON p_tgt.id = f2.package_id
             WHERE p_src.id != p_tgt.id
             GROUP BY p_src.name, p_tgt.name, e.kind",
        )
        .context("Failed to prepare workspace_graph edges query")?;

    let edge_rows = edge_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u32>(3)?,
            ))
        })
        .context("Failed to execute workspace_graph edges query")?;

    // Aggregate into a map keyed by (src, tgt).
    let mut acc: HashMap<(String, String), WorkspaceGraphEdge> = HashMap::new();
    for row in edge_rows {
        let (src, tgt, kind, cnt) = row.context("row fetch failed")?;
        let entry = acc
            .entry((src.clone(), tgt.clone()))
            .or_insert_with(|| WorkspaceGraphEdge {
                source_package: src,
                target_package: tgt,
                code_edges: 0,
                code_by_kind: Vec::new(),
                flow_edges: 0,
                flow_by_kind: Vec::new(),
                declared_dep: false,
                total_edges: 0,
            });
        if is_flow(&kind) {
            entry.flow_edges += cnt;
            entry.flow_by_kind.push((kind, cnt));
        } else {
            entry.code_edges += cnt;
            entry.code_by_kind.push((kind, cnt));
        }
        entry.total_edges += cnt;
    }

    // --- Declared deps (manifest-level) ---
    //
    // `package_deps.dep_name` is whatever the manifest wrote. Match against
    // both `declared_name` and `name` on the target package so that workspaces
    // using folder-name references (Cargo path-deps without a manifest name)
    // and workspaces using declared names both land here.
    let mut dep_stmt = conn
        .prepare_cached(
            "SELECT DISTINCT p_src.name, p_tgt.name
             FROM package_deps pd
             JOIN packages p_src ON p_src.id = pd.package_id
             JOIN packages p_tgt
                 ON p_tgt.declared_name = pd.dep_name
                 OR p_tgt.name          = pd.dep_name
             WHERE p_src.id != p_tgt.id",
        )
        .context("Failed to prepare workspace_graph deps query")?;

    let dep_rows = dep_stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("Failed to execute workspace_graph deps query")?;

    for row in dep_rows {
        let (src, tgt) = row.context("dep row fetch failed")?;
        let entry = acc
            .entry((src.clone(), tgt.clone()))
            .or_insert_with(|| WorkspaceGraphEdge {
                source_package: src,
                target_package: tgt,
                code_edges: 0,
                code_by_kind: Vec::new(),
                flow_edges: 0,
                flow_by_kind: Vec::new(),
                declared_dep: false,
                total_edges: 0,
            });
        entry.declared_dep = true;
    }

    // Stable output: sort by total edges desc, then src/tgt name.
    let mut out: Vec<WorkspaceGraphEdge> = acc.into_values().collect();
    for e in &mut out {
        e.code_by_kind.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        e.flow_by_kind.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    }
    out.sort_by(|a, b| {
        b.total_edges
            .cmp(&a.total_edges)
            .then_with(|| a.source_package.cmp(&b.source_package))
            .then_with(|| a.target_package.cmp(&b.target_package))
    });

    Ok(out)
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

    /// Extended fixture: two packages, one calls edge + one http_call edge +
    /// a manifest-declared dependency. Exercises the workspace_graph merge.
    fn setup_graph_fixture(db: &Database) {
        let conn = db.conn();
        conn.execute(
            "INSERT INTO packages (name, path, kind, declared_name) VALUES ('pkg-a', 'packages/a', 'npm', '@myorg/a')",
            [],
        ).unwrap();
        let pkg_a = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO packages (name, path, kind, declared_name) VALUES ('pkg-b', 'packages/b', 'npm', '@myorg/b')",
            [],
        ).unwrap();
        let pkg_b = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed, package_id) VALUES ('packages/a/src/a.ts', 'h1', 'typescript', 0, ?1)",
            rusqlite::params![pkg_a],
        ).unwrap();
        let file_a = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed, package_id) VALUES ('packages/b/src/b.ts', 'h2', 'typescript', 0, ?1)",
            rusqlite::params![pkg_b],
        ).unwrap();
        let file_b = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'fn_a', 'a.fn_a', 'function', 1, 0)",
            rusqlite::params![file_a],
        ).unwrap();
        let sym_a = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) VALUES (?1, 'fn_b', 'b.fn_b', 'function', 1, 0)",
            rusqlite::params![file_b],
        ).unwrap();
        let sym_b = conn.last_insert_rowid();

        // Two calls edges + one http_call edge from A → B.
        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) VALUES (?1, ?2, 'calls', 1, 1.0)",
            rusqlite::params![sym_a, sym_b],
        ).unwrap();
        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) VALUES (?1, ?2, 'calls', 2, 1.0)",
            rusqlite::params![sym_a, sym_b],
        ).unwrap();
        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) VALUES (?1, ?2, 'http_call', 3, 0.9)",
            rusqlite::params![sym_a, sym_b],
        ).unwrap();

        // Manifest-declared dep: pkg-a's package.json lists @myorg/b.
        conn.execute(
            "INSERT INTO package_deps (package_id, ecosystem, dep_name, version, kind)
             VALUES (?1, 'typescript', '@myorg/b', '^1.0.0', 'runtime')",
            rusqlite::params![pkg_a],
        ).unwrap();
    }

    #[test]
    fn workspace_graph_aggregates_code_flow_and_declared() {
        let db = Database::open_in_memory().unwrap();
        setup_graph_fixture(&db);

        let edges = workspace_graph(&db).unwrap();
        assert_eq!(edges.len(), 1);
        let e = &edges[0];
        assert_eq!(e.source_package, "pkg-a");
        assert_eq!(e.target_package, "pkg-b");
        assert_eq!(e.code_edges, 2);
        assert_eq!(e.flow_edges, 1);
        assert_eq!(e.total_edges, 3);
        assert!(e.declared_dep);
        assert_eq!(e.code_by_kind, vec![("calls".to_string(), 2)]);
        assert_eq!(e.flow_by_kind, vec![("http_call".to_string(), 1)]);
    }

    #[test]
    fn workspace_graph_surfaces_declared_dep_with_zero_edges() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO packages (name, path, kind, declared_name) VALUES ('x', 'x', 'npm', '@acme/x')",
            [],
        ).unwrap();
        let x = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO packages (name, path, kind, declared_name) VALUES ('y', 'y', 'npm', '@acme/y')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO package_deps (package_id, ecosystem, dep_name, version, kind)
             VALUES (?1, 'typescript', '@acme/y', '1', 'runtime')",
            rusqlite::params![x],
        ).unwrap();

        let edges = workspace_graph(&db).unwrap();
        assert_eq!(edges.len(), 1);
        assert!(edges[0].declared_dep);
        assert_eq!(edges[0].total_edges, 0);
        assert_eq!(edges[0].source_package, "x");
        assert_eq!(edges[0].target_package, "y");
    }

    #[test]
    fn workspace_graph_empty_for_single_project() {
        let db = Database::open_in_memory().unwrap();
        let edges = workspace_graph(&db).unwrap();
        assert!(edges.is_empty());
    }

    #[test]
    fn workspace_graph_matches_dep_name_by_folder_fallback() {
        // Cargo path-deps sometimes list the folder name rather than a
        // separate declared_name. Verify the fallback match still fires.
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO packages (name, path, kind) VALUES ('core', 'crates/core', 'cargo')",
            [],
        ).unwrap();
        let core = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO packages (name, path, kind) VALUES ('cli', 'crates/cli', 'cargo')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO package_deps (package_id, ecosystem, dep_name, version, kind)
             VALUES (?1, 'rust', 'cli', NULL, 'runtime')",
            rusqlite::params![core],
        ).unwrap();

        let edges = workspace_graph(&db).unwrap();
        assert_eq!(edges.len(), 1);
        assert!(edges[0].declared_dep);
        assert_eq!(edges[0].source_package, "core");
        assert_eq!(edges[0].target_package, "cli");
    }
}
