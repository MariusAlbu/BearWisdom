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
    /// Refs originating in this package that resolved with confidence ≥ 0.5.
    /// This is the strong-resolution count; `edge_count` above includes all
    /// edges (low-confidence + flow + LSP-imported).
    pub resolved_refs: u32,
    /// Refs originating in this package that fell into `unresolved_refs`.
    pub unresolved_refs: u32,
    /// Resolution rate: `resolved_refs / (resolved_refs + unresolved_refs)`,
    /// `None` when the package emitted zero refs.
    pub resolved_pct: Option<f32>,
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
///
/// `source_package` / `target_package` carry display names. `*_path` and
/// `*_kind` disambiguate when two packages share a display name — e.g. a
/// monorepo with `apps/core/package.json` and `crates/core/Cargo.toml` both
/// named `core`. Consumers that key on identity should use `(path, kind)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageDependency {
    pub source_package: String,
    pub target_package: String,
    pub source_package_path: String,
    pub target_package_path: String,
    pub source_package_kind: Option<String>,
    pub target_package_kind: Option<String>,
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
    /// Path of the source package, relative to the project root. Disambiguates
    /// when two packages share `source_package`.
    pub source_package_path: String,
    /// Path of the target package, relative to the project root.
    pub target_package_path: String,
    /// Manifest kind of the source package (`"cargo"`, `"npm"`, ...). `None`
    /// for unclassified packages.
    pub source_package_kind: Option<String>,
    /// Manifest kind of the target package.
    pub target_package_kind: Option<String>,
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
    /// (either by `declared_name` or by folder-derived `name`) AND the
    /// manifest's ecosystem agrees with the target package's kind. The
    /// ecosystem check prevents a cross-language `shared` package from
    /// matching every `shared` in the workspace.
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
    // Stats are scoped to `origin = 'internal'` so vendored / external
    // sources written under `ext:` paths don't inflate per-package size
    // and cross-package coupling counts. `resolved_refs` and
    // `unresolved_refs` use the same edge-kind universe — only ref-like
    // edges (`unresolved_refs` rows have kinds calls/type_ref/inherits/
    // implements/instantiates/imports). Without the kind filter on
    // `resolved_refs`, non-ref edges (lsp_resolved, http_call, db_entity)
    // entered the numerator but never the denominator, so `resolved_pct`
    // could exceed 100% or float above the real resolution rate.
    let mut stmt = conn
        .prepare_cached(
            "SELECT p.name,
                    p.path,
                    p.kind,
                    (SELECT COUNT(*) FROM files f
                     WHERE f.package_id = p.id AND f.origin = 'internal')
                        AS file_count,
                    (SELECT COUNT(*) FROM symbols s
                     JOIN files f ON f.id = s.file_id
                     WHERE f.package_id = p.id AND f.origin = 'internal')
                        AS symbol_count,
                    (SELECT COUNT(*) FROM edges e
                     JOIN symbols s ON s.id = e.source_id
                     JOIN files   f ON f.id = s.file_id
                     WHERE f.package_id = p.id AND f.origin = 'internal')
                        AS edge_count,
                    (SELECT COUNT(*) FROM edges e
                     JOIN symbols s ON s.id = e.source_id
                     JOIN files   f ON f.id = s.file_id
                     WHERE f.package_id = p.id
                       AND f.origin = 'internal'
                       AND e.confidence >= 0.5
                       AND e.kind IN ('calls','type_ref','inherits',
                                      'implements','instantiates','imports'))
                        AS resolved_refs,
                    (SELECT COUNT(*) FROM unresolved_refs u
                     JOIN symbols s ON s.id = u.source_id
                     JOIN files   f ON f.id = s.file_id
                     WHERE f.package_id = p.id AND f.origin = 'internal')
                        AS unresolved_refs
             FROM packages p
             ORDER BY file_count DESC, p.name",
        )
        .context("Failed to prepare list_packages query")?;

    let rows = stmt
        .query_map([], |row| {
            let resolved: u32 = row.get::<_, u32>(6).unwrap_or(0);
            let unresolved: u32 = row.get::<_, u32>(7).unwrap_or(0);
            let total = resolved + unresolved;
            let resolved_pct = if total > 0 {
                Some(resolved as f32 / total as f32)
            } else {
                None
            };
            Ok(PackageStats {
                name:            row.get(0)?,
                path:            row.get(1)?,
                kind:            row.get(2)?,
                file_count:      row.get::<_, u32>(3).unwrap_or(0),
                symbol_count:    row.get::<_, u32>(4).unwrap_or(0),
                edge_count:      row.get::<_, u32>(5).unwrap_or(0),
                resolved_refs:   resolved,
                unresolved_refs: unresolved,
                resolved_pct,
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
               AND f1.package_id != f2.package_id
               AND f1.origin = 'internal'
               AND f2.origin = 'internal'",
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
               AND f_src.origin = 'internal'
               AND f_tgt.origin = 'internal'
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
                    p_src.path,
                    p_tgt.path,
                    p_src.kind,
                    p_tgt.kind,
                    COUNT(*)          AS edge_count
             FROM edges e
             JOIN symbols s1  ON e.source_id   = s1.id
             JOIN files   f1  ON s1.file_id    = f1.id
             JOIN packages p_src ON f1.package_id = p_src.id
             JOIN symbols s2  ON e.target_id   = s2.id
             JOIN files   f2  ON s2.file_id    = f2.id
             JOIN packages p_tgt ON f2.package_id = p_tgt.id
             WHERE p_src.id != p_tgt.id
               AND f1.origin = 'internal'
               AND f2.origin = 'internal'
             GROUP BY p_src.id, p_tgt.id
             ORDER BY edge_count DESC",
        )
        .context("Failed to prepare package_dependencies query")?;

    let rows = stmt
        .query_map([], |row| {
            Ok(PackageDependency {
                source_package:      row.get(0)?,
                target_package:      row.get(1)?,
                source_package_path: row.get(2)?,
                target_package_path: row.get(3)?,
                source_package_kind: row.get(4)?,
                target_package_kind: row.get(5)?,
                edge_count:          row.get(6)?,
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
            "SELECT p_src.id,
                    p_tgt.id,
                    p_src.name,
                    p_tgt.name,
                    p_src.path,
                    p_tgt.path,
                    p_src.kind,
                    p_tgt.kind,
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
               AND f1.origin = 'internal'
               AND f2.origin = 'internal'
             GROUP BY p_src.id, p_tgt.id, e.kind",
        )
        .context("Failed to prepare workspace_graph edges query")?;

    let edge_rows = edge_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, u32>(9)?,
            ))
        })
        .context("Failed to execute workspace_graph edges query")?;

    // Aggregate into a map keyed by (src_id, tgt_id). Identity is the
    // package row, not the display name — two packages can legitimately
    // share `name` (e.g. polyglot `core` in apps/ and crates/).
    let mut acc: HashMap<(i64, i64), WorkspaceGraphEdge> = HashMap::new();
    for row in edge_rows {
        let (src_id, tgt_id, src_name, tgt_name, src_path, tgt_path, src_kind, tgt_kind, kind, cnt) =
            row.context("row fetch failed")?;
        let entry = acc.entry((src_id, tgt_id)).or_insert_with(|| WorkspaceGraphEdge {
            source_package: src_name,
            target_package: tgt_name,
            source_package_path: src_path,
            target_package_path: tgt_path,
            source_package_kind: src_kind,
            target_package_kind: tgt_kind,
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
    // Match `package_deps.dep_name` against both `declared_name` and `name`
    // on the target package (so workspaces using folder names and ones using
    // declared names both land). The ecosystem clause prevents a TS package
    // declaring `shared` from binding to a Cargo `shared` crate in the same
    // workspace — distinct ecosystems shouldn't fuse on a name collision.
    let mut dep_stmt = conn
        .prepare_cached(
            "SELECT DISTINCT
                    p_src.id, p_tgt.id,
                    p_src.name, p_tgt.name,
                    p_src.path, p_tgt.path,
                    p_src.kind, p_tgt.kind
             FROM package_deps pd
             JOIN packages p_src ON p_src.id = pd.package_id
             JOIN packages p_tgt
                 ON (p_tgt.declared_name = pd.dep_name OR p_tgt.name = pd.dep_name)
                AND pd.ecosystem = CASE p_tgt.kind
                        WHEN 'npm'     THEN 'typescript'
                        WHEN 'cargo'   THEN 'rust'
                        WHEN 'go'      THEN 'go'
                        WHEN 'python'  THEN 'python'
                        WHEN 'java'    THEN 'java'
                        WHEN 'ruby'    THEN 'ruby'
                        WHEN 'php'     THEN 'php'
                        WHEN 'swift'   THEN 'swift'
                        WHEN 'dart'    THEN 'dart'
                        WHEN 'elixir'  THEN 'elixir'
                        WHEN 'dotnet'  THEN 'dotnet'
                        WHEN 'r'       THEN 'r'
                        WHEN 'scala'   THEN 'scala'
                        WHEN 'ocaml'   THEN 'ocaml'
                        ELSE pd.ecosystem
                    END
             WHERE p_src.id != p_tgt.id",
        )
        .context("Failed to prepare workspace_graph deps query")?;

    let dep_rows = dep_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .context("Failed to execute workspace_graph deps query")?;

    for row in dep_rows {
        let (src_id, tgt_id, src_name, tgt_name, src_path, tgt_path, src_kind, tgt_kind) =
            row.context("dep row fetch failed")?;
        let entry = acc.entry((src_id, tgt_id)).or_insert_with(|| WorkspaceGraphEdge {
            source_package: src_name,
            target_package: tgt_name,
            source_package_path: src_path,
            target_package_path: tgt_path,
            source_package_kind: src_kind,
            target_package_kind: tgt_kind,
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
#[path = "workspace_tests.rs"]
mod tests;

