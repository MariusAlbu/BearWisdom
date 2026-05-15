// =============================================================================
// query/dead_code.rs — Dead code discovery and entry point inference
//
// Identifies symbols with zero incoming edges that are NOT entry points.
// Entry points are: main functions, route handlers, event handlers, test
// functions, exported library API, and framework lifecycle hooks.
//
// Dead code candidates are scored by confidence:
//   1.0 — private symbol, 0 incoming, not entry point, not in test file
//   0.9 — internal symbol, 0 incoming
//   0.7 — public symbol in an application, 0 incoming
//   0.5 — public symbol in a library, 0 incoming (may be API surface)
//   0.3 — symbol has only low-confidence edges (<0.7)
// =============================================================================

use crate::db::Database;
use crate::query::QueryResult;
use anyhow::Context;
use serde::{Deserialize, Serialize};

// Re-export the entry-point types & report function from their new
// home so existing callers (`bearwisdom-cli`, `bearwisdom-mcp`,
// `bearwisdom-web`, `bearwisdom-mcp/compact`) keep working unchanged.
pub use crate::query::entry_points::{
    EntryPoint, EntryPointKind, EntryPointsReport, find_entry_points,
};

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Filter for which visibility levels to include.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityFilter {
    /// Only private/internal symbols (highest confidence dead code).
    PrivateOnly,
    /// Only public symbols (potential API surface — lower confidence).
    PublicOnly,
    /// All visibility levels.
    #[default]
    All,
}

/// Options controlling dead code discovery.
#[derive(Debug, Clone)]
pub struct DeadCodeOptions {
    /// Restrict to a file path, directory prefix, or package name.
    pub scope: Option<String>,
    /// Which visibility levels to include.
    pub visibility_filter: VisibilityFilter,
    /// Include symbols in test files (default: false).
    pub include_tests: bool,
    /// Which symbol kinds to check (empty = all meaningful kinds).
    pub kinds: Vec<String>,
    /// Maximum results to return (default: 100).
    pub max_results: usize,
}

impl Default for DeadCodeOptions {
    fn default() -> Self {
        Self {
            scope: None,
            visibility_filter: VisibilityFilter::default(),
            include_tests: false,
            kinds: Vec::new(),
            max_results: 100,
        }
    }
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Why a symbol was flagged as dead code.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadCodeReason {
    /// Zero incoming edges and not an entry point.
    NoIncomingEdges,
    /// Only low-confidence edges (heuristic guesses, <0.7).
    OnlyLowConfidenceEdges,
}

/// A single dead code candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeEntry {
    pub symbol_id: i64,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub visibility: Option<String>,
    pub file_path: String,
    pub line: u32,
    /// 0.0–1.0 how likely this is truly dead code.
    pub confidence: f64,
    pub reason: DeadCodeReason,
    /// True if this symbol's name appears as a target in `unresolved_refs`,
    /// meaning something tried to reference it but the resolver couldn't connect
    /// the dots. Treat with caution — may NOT be dead.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub potentially_referenced: bool,
    /// Number of unresolved refs matching this symbol's name (from the same file
    /// or via qualified name). Only set when `potentially_referenced` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unresolved_ref_matches: Option<u32>,
    /// Chain of edge kinds from the nearest entry point on this candidate's
    /// best "alive" path, if one exists below the reachability threshold.
    /// Always `None` in Phase 3 — populated when Phase 4 introduces
    /// dispatch_candidate / construct / reflection_root synthesis so callers
    /// can surface "alive only via synthesized dispatch" hits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reachability_path: Option<Vec<String>>,
    /// BFS hop count from the nearest entry point, or `None` if unreachable.
    /// For dead candidates this is always `None` today; Phase 4 widens the
    /// threshold and lights it up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_point_distance: Option<u32>,
}

/// Trust tier for a dead-code report. Wires the resolution-gate trust
/// model from `research/ArchitectureImprovements/Codex/01-resolution-gate-plan.md`.
///
/// In `Unsafe`, high-confidence deletion recommendations are suppressed —
/// candidate confidences are clamped so callers can't act on them as if
/// they were ground truth.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrustTier {
    /// Internal resolution >= 99% AND low-confidence edges < 5% of resolved
    /// edges. Dead-code candidates are actionable.
    Trusted,
    /// Internal resolution between 95% and 99%, or low-confidence edges
    /// 5%-15%. Dead-code candidates need human review before deletion.
    Review,
    /// Internal resolution below 95%, or low-confidence edges > 15%.
    /// Dead-code report is informational only; high-confidence
    /// recommendations are suppressed.
    Unsafe,
}

impl TrustTier {
    pub fn as_str(self) -> &'static str {
        match self {
            TrustTier::Trusted => "trusted",
            TrustTier::Review => "review",
            TrustTier::Unsafe => "unsafe",
        }
    }
}

/// Resolution health — tells the user how trustworthy the results are.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionHealth {
    /// Percentage of refs that were resolved or classified as external.
    pub resolution_rate: f64,
    /// Total resolved + external refs.
    pub resolved_refs: u64,
    /// Total unresolved refs remaining.
    pub unresolved_refs: u64,
    /// Count of resolved edges with confidence below the heuristic
    /// threshold (0.8 by default). Distinct from unresolved — these are
    /// edges that resolved, but only via best-guess strategies.
    pub low_confidence_edges: u64,
    /// Trust tier used to gate dead-code recommendations.
    pub trust_tier: TrustTier,
    /// Human-readable assessment.
    pub assessment: String,
}

/// Full dead code report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeReport {
    pub total_symbols_checked: u32,
    pub dead_candidates: Vec<DeadCodeEntry>,
    pub entry_points_excluded: u32,
    pub test_symbols_excluded: u32,
    /// How many candidates were flagged as `potentially_referenced` due to
    /// matching unresolved refs. These should be reviewed carefully.
    pub potentially_referenced_count: u32,
    /// Overall resolution health — indicates how trustworthy this report is.
    pub resolution_health: ResolutionHealth,
}

// ---------------------------------------------------------------------------
// Dead code discovery
// ---------------------------------------------------------------------------

/// Find dead code candidates — symbols with zero (or only low-confidence)
/// incoming edges that are not entry points.
pub fn find_dead_code(
    db: &Database,
    options: &DeadCodeOptions,
) -> QueryResult<DeadCodeReport> {
    let _timer = db.timer("dead_code");
    let conn = db.conn();

    // --- Resolution health ---
    // Scope resolution: turn the optional scope string into a set of file_ids
    // covering matched paths AND packages whose declared_name / folder name /
    // path matches the scope. `None` means whole-project. Used by both
    // candidate filtering, unresolved-ref matching, and resolution-health
    // computation so all three see the same scope.
    let scope_file_ids = resolve_scope_file_ids(conn, options.scope.as_deref())?;

    let resolution_health = compute_resolution_health(conn, scope_file_ids.as_ref())?;

    // --- Build unresolved ref targets for cross-referencing ---
    // Maps (target_name) → count of unresolved refs with that target.
    // Scope-aware: a name collision between two packages in a monorepo
    // shouldn't keep a dead symbol alive just because another package's
    // resolver missed something with the same name.
    let unresolved_names = build_unresolved_name_counts(conn, scope_file_ids.as_ref())?;

    // L2+L1+L3+L4 of the reachability stack. Lazy: ensures the dispatch
    // synthesizer, contributors, BFS, and per-package health table have
    // all run even when the caller bypassed `finalize_resolution` (test
    // fixtures, partial pipelines). Phase 3+4+7 wired the same calls
    // into finalize_resolution for production indexes; running them here
    // a second time after a fresh reindex is cheap (each step is
    // idempotent and the work is dominated by the row counts in the
    // tables they read).
    crate::indexer::resolve::synthesize_dispatch::synthesize_dispatch_edges(db)?;
    crate::query::entry_points::rebuild_entry_points(db)?;
    crate::indexer::resolve::reachability::materialize_reachability(db)?;
    materialize_package_resolution_health(db)?;

    let test_file_ids = collect_test_file_ids(conn)?;

    // Count how many entry-point symbols anchor reachability — informational
    // only, surfaces "this many roots seed the BFS" in the report. The old
    // `entry_points_excluded` counter (incremented when an entry point also
    // had zero incoming edges) no longer makes sense under reachability
    // semantics — entry points are inherently reachable so they never
    // appear in the dead antijoin to begin with.
    let entry_points_count: u32 = conn
        .query_row(
            "SELECT COUNT(DISTINCT symbol_id) FROM entry_points",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as u32;

    // Per-package resolution-health multipliers feed the L4 confidence
    // formula: a candidate in a struggling package gets its confidence
    // dampened so high-resolution-rate packages keep their actionable
    // signal even when other packages drag the global trust tier down.
    let pkg_health = load_package_health_multipliers(conn)?;
    let file_to_pkg = load_file_to_package_map(conn)?;

    // Default kinds to check.
    let default_kinds = [
        "function", "method", "class", "struct", "interface", "enum",
        "type_alias", "trait", "protocol",
    ];
    let check_kinds: Vec<&str> = if options.kinds.is_empty() {
        default_kinds.to_vec()
    } else {
        options.kinds.iter().map(|s| s.as_str()).collect()
    };

    // Build the antijoin query. A symbol is a dead candidate iff it has
    // no row in `reachability` — i.e. the BFS from entry points never
    // reached it through edges above the confidence threshold.
    let mut sql = String::from(
        "SELECT s.id, s.name, s.qualified_name, s.kind, s.visibility, \
                f.path, s.line, f.id as file_id \
         FROM symbols s \
         JOIN files f ON f.id = s.file_id \
         LEFT JOIN reachability r ON r.symbol_id = s.id \
         WHERE r.symbol_id IS NULL \
           AND s.origin = 'internal'",
    );

    // Kind filter — use placeholders.
    let kind_placeholders: Vec<String> = check_kinds
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect();
    sql.push_str(&format!(
        " AND s.kind IN ({})",
        kind_placeholders.join(", ")
    ));

    // Visibility filter.
    match options.visibility_filter {
        VisibilityFilter::PrivateOnly => {
            sql.push_str(" AND (s.visibility = 'private' OR s.visibility IS NULL)");
        }
        VisibilityFilter::PublicOnly => {
            sql.push_str(" AND s.visibility = 'public'");
        }
        VisibilityFilter::All => {}
    }

    // Scope filter.
    if let Some(ids) = &scope_file_ids {
        if ids.is_empty() {
            sql.push_str(" AND 0 = 1");
        } else {
            let csv = ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            sql.push_str(&format!(" AND f.id IN ({})", csv));
        }
    }

    // No SQL LIMIT — sort happens in Rust after confidence is computed so
    // the top-N by confidence wins instead of top-N by file path (the old
    // SQL-LIMIT-then-sort bug).
    sql.push_str(" ORDER BY f.path, s.line");

    let mut stmt = conn.prepare(&sql).context("dead_code: prepare query")?;

    let params: Vec<Box<dyn rusqlite::types::ToSql>> = check_kinds
        .iter()
        .map(|k| Box::new(k.to_string()) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(|p| p.as_ref()).collect();

    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,            // id
                row.get::<_, String>(1)?,          // name
                row.get::<_, String>(2)?,          // qualified_name
                row.get::<_, String>(3)?,          // kind
                row.get::<_, Option<String>>(4)?, // visibility
                row.get::<_, String>(5)?,          // path
                row.get::<_, u32>(6)?,             // line
                row.get::<_, i64>(7)?,             // file_id
            ))
        })
        .context("dead_code: execute query")?;

    let mut candidates = Vec::new();
    let mut total_checked: u32 = 0;
    let mut test_symbols_excluded: u32 = 0;

    for row in rows {
        let (id, name, qname, kind, visibility, path, line, file_id) =
            row.context("dead_code: read row")?;

        total_checked += 1;

        // Test files excluded by default; reachability already filters
        // test contributors out of its seed set, so test FUNCTIONS in
        // non-test files are still candidates unless the user opts in.
        if !options.include_tests && test_file_ids.contains(&file_id) {
            test_symbols_excluded += 1;
            continue;
        }

        if is_noise_symbol(&name, &kind) {
            continue;
        }

        let mut confidence = match visibility.as_deref() {
            Some("private") | None => 1.0,
            Some("internal") => 0.9,
            Some("public") => 0.7,
            _ => 0.8,
        };

        // L4: per-package resolution-health multiplier. A candidate in a
        // package whose resolver only resolved 60% of refs is much less
        // trustworthy than one in a 99%-resolved package — even if the
        // global trust tier hasn't tipped over to Unsafe.
        if let Some(pid) = file_to_pkg.get(&file_id) {
            if let Some(&mult) = pkg_health.get(pid) {
                confidence *= mult;
            }
        }

        // Cross-reference against unresolved refs — same heuristic as
        // before: if something tried to reference this symbol's name and
        // the resolver couldn't pin it down, treat the candidate with
        // suspicion (halve confidence + flag).
        let unresolved_match_count = unresolved_names
            .get(qname.as_str())
            .or_else(|| {
                if !is_generic_name(&name) {
                    unresolved_names.get(name.as_str())
                } else {
                    None
                }
            })
            .copied()
            .unwrap_or(0);

        let potentially_referenced = unresolved_match_count > 0;
        if potentially_referenced {
            confidence *= 0.5;
        }

        candidates.push(DeadCodeEntry {
            symbol_id: id,
            name,
            qualified_name: qname,
            kind,
            visibility,
            file_path: path,
            line,
            confidence,
            reason: DeadCodeReason::NoIncomingEdges,
            potentially_referenced,
            unresolved_ref_matches: if potentially_referenced {
                Some(unresolved_match_count)
            } else {
                None
            },
            reachability_path: None,
            entry_point_distance: None,
        });
    }

    // Trust-tier clamp — same semantics as v0: in Unsafe, cap every
    // candidate at 0.5 so callers can't treat the report as ground truth.
    if resolution_health.trust_tier == TrustTier::Unsafe {
        for c in &mut candidates {
            if c.confidence > 0.5 {
                c.confidence = 0.5;
            }
        }
    }

    // Sort BEFORE truncating to max_results — gives the user the
    // highest-confidence candidates, not the alphabetically-first ones.
    candidates.sort_by(|a, b| {
        a.potentially_referenced
            .cmp(&b.potentially_referenced)
            .then_with(|| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    if candidates.len() > options.max_results {
        candidates.truncate(options.max_results);
    }

    let potentially_referenced_count = candidates
        .iter()
        .filter(|c| c.potentially_referenced)
        .count() as u32;

    Ok(DeadCodeReport {
        total_symbols_checked: total_checked,
        dead_candidates: candidates,
        entry_points_excluded: entry_points_count,
        test_symbols_excluded,
        potentially_referenced_count,
        resolution_health,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers (entry-point discovery lives in crate::query::entry_points)
// ---------------------------------------------------------------------------

/// Collect file IDs that are test files.
fn collect_test_file_ids(
    conn: &rusqlite::Connection,
) -> QueryResult<std::collections::HashSet<i64>> {
    let mut ids = std::collections::HashSet::new();
    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE origin = 'internal'")
        .context("test_file_ids: prepare")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .context("test_file_ids: query")?;
    for row in rows.flatten() {
        if crate::indexer::test_file_detection::is_test_file(&row.1) {
            ids.insert(row.0);
        }
    }
    Ok(ids)
}

/// Skip symbols that are noise — constructors, getters/setters, operators, etc.
fn is_noise_symbol(name: &str, kind: &str) -> bool {
    // Property accessors (C# get/set, Kotlin get/set)
    if kind == "method" && matches!(name, "get" | "set") {
        return true;
    }
    matches!(
        name,
        "constructor"
            | "new"
            | "init"
            | "toString"
            | "hashCode"
            | "equals"
            | "clone"
            | "finalize"
            | "compareTo"
            | "Equals"
            | "GetHashCode"
            | "ToString"
            | "Dispose"
            | "Finalize"
            | "__init__"
            | "__str__"
            | "__repr__"
            | "__eq__"
            | "__hash__"
            | "__del__"
            | "__enter__"
            | "__exit__"
    )
}

/// Compute resolution health for the dead-code trust tier.
///
/// `scope_file_ids = None` returns project-wide health (the original
/// behavior). `Some(ids)` scopes every count to refs originating in
/// those files, so a single-package report inside a monorepo gets a
/// trust tier reflecting that package's resolver coverage and not the
/// workspace average.
///
/// Uses the same metric the resolution-gate plan defines:
/// `internal_edges / (internal_edges + internal_unresolved)`, restricted
/// to first-party (`origin = 'internal'`) source. Doc-snippet refs are
/// excluded via the same `CODE_REF_FILTER` the `resolution_breakdown`
/// query uses, so the headline rate matches the gate metric exactly.
fn compute_resolution_health(
    conn: &rusqlite::Connection,
    scope_file_ids: Option<&std::collections::HashSet<i64>>,
) -> QueryResult<ResolutionHealth> {
    let scope_clause = match scope_file_ids {
        None => String::new(),
        Some(ids) if ids.is_empty() => " AND 0 = 1".to_string(),
        Some(ids) => format!(
            " AND f.id IN ({})",
            ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",")
        ),
    };

    let edges_sql = format!(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols s ON s.id = e.source_id
         JOIN files   f ON f.id = s.file_id
         WHERE f.origin = 'internal'{scope_clause}"
    );
    let internal_edges: u64 = conn
        .query_row(&edges_sql, [], |row| row.get(0))
        .unwrap_or(0);

    let internal_unresolved_sql = format!(
        "SELECT COUNT(*)
         FROM unresolved_refs u
         JOIN symbols s ON s.id = u.source_id
         JOIN files   f ON f.id = s.file_id
         WHERE f.origin = 'internal' AND {filter}{scope_clause}",
        filter = crate::query::stats::CODE_REF_FILTER
    );
    let internal_unresolved: u64 = conn
        .query_row(&internal_unresolved_sql, [], |row| row.get(0))
        .unwrap_or(0);

    let low_conf_threshold = crate::query::diagnostics::LOW_CONFIDENCE_THRESHOLD;
    let low_conf_sql = format!(
        "SELECT COUNT(*)
         FROM edges e
         JOIN symbols s ON s.id = e.source_id
         JOIN files   f ON f.id = s.file_id
         WHERE f.origin = 'internal' AND e.confidence < ?1{scope_clause}"
    );
    let low_confidence_edges: u64 = conn
        .query_row(&low_conf_sql, [low_conf_threshold], |row| row.get(0))
        .unwrap_or(0);

    let total = internal_edges + internal_unresolved;
    let rate = if total > 0 {
        (internal_edges as f64 / total as f64) * 100.0
    } else {
        100.0
    };
    let rate = (rate * 10.0).round() / 10.0;

    // Low-confidence-edge ratio drives the trust-tier as a second axis
    // beyond the headline rate. A project that resolved everything via
    // heuristics still doesn't have ground-truth dead-code answers.
    let low_conf_ratio = if internal_edges > 0 {
        low_confidence_edges as f64 / internal_edges as f64
    } else {
        0.0
    };

    let trust_tier = if rate >= 99.0 && low_conf_ratio < 0.05 {
        TrustTier::Trusted
    } else if rate >= 95.0 && low_conf_ratio < 0.15 {
        TrustTier::Review
    } else {
        TrustTier::Unsafe
    };

    let assessment = match trust_tier {
        TrustTier::Trusted => format!(
            "Trusted — {rate:.1}% internal resolution, {:.1}% low-confidence \
             edges. Dead-code candidates are actionable.",
            low_conf_ratio * 100.0
        ),
        TrustTier::Review => format!(
            "Review — {rate:.1}% internal resolution, {:.1}% low-confidence \
             edges. Dead-code candidates need human review before deletion.",
            low_conf_ratio * 100.0
        ),
        TrustTier::Unsafe => format!(
            "Unsafe — {rate:.1}% internal resolution, {:.1}% low-confidence \
             edges. Dead-code report is informational only; high-confidence \
             recommendations are suppressed.",
            low_conf_ratio * 100.0
        ),
    };

    Ok(ResolutionHealth {
        resolution_rate: rate,
        resolved_refs: internal_edges,
        unresolved_refs: internal_unresolved,
        low_confidence_edges,
        trust_tier,
        assessment,
    })
}

/// Materialize per-package resolution health into the
/// `package_resolution_health` table. One row per workspace package.
/// Folded into `find_dead_code`'s confidence calculation so a single
/// struggling package's candidates get dampened without pulling the
/// whole workspace into the Unsafe trust tier.
///
/// Wired into `resolve::finalize_resolution` alongside reachability.
/// Idempotent — `INSERT OR REPLACE` keyed on `package_id`.
pub fn materialize_package_resolution_health(db: &Database) -> QueryResult<()> {
    let conn = db.conn();
    let low_conf_threshold = crate::query::diagnostics::LOW_CONFIDENCE_THRESHOLD;

    // Iterate packages; per-package counts via parameterized queries
    // (one prepare + N executes). For a 100-package workspace this is
    // <10ms — cheap relative to the BFS materialization.
    let mut pkg_stmt = conn
        .prepare("SELECT id FROM packages")
        .context("pkg_health: list packages")?;
    let package_ids: Vec<i64> = pkg_stmt
        .query_map([], |r| r.get::<_, i64>(0))?
        .filter_map(|r| r.ok())
        .collect();

    let mut edges_stmt = conn
        .prepare(
            "SELECT COUNT(*) FROM edges e \
             JOIN symbols s ON s.id = e.source_id \
             JOIN files   f ON f.id = s.file_id \
             WHERE f.origin = 'internal' AND f.package_id = ?1",
        )
        .context("pkg_health: prepare edges count")?;
    let mut unresolved_stmt = conn
        .prepare(&format!(
            "SELECT COUNT(*) FROM unresolved_refs u \
             JOIN symbols s ON s.id = u.source_id \
             JOIN files   f ON f.id = s.file_id \
             WHERE f.origin = 'internal' AND f.package_id = ?1 AND {filter}",
            filter = crate::query::stats::CODE_REF_FILTER
        ))
        .context("pkg_health: prepare unresolved count")?;
    let mut low_conf_stmt = conn
        .prepare(
            "SELECT COUNT(*) FROM edges e \
             JOIN symbols s ON s.id = e.source_id \
             JOIN files   f ON f.id = s.file_id \
             WHERE f.origin = 'internal' AND f.package_id = ?1 AND e.confidence < ?2",
        )
        .context("pkg_health: prepare low_conf count")?;

    let tx = conn
        .unchecked_transaction()
        .context("pkg_health: begin transaction")?;
    tx.execute("DELETE FROM package_resolution_health", [])
        .context("pkg_health: clear table")?;
    let mut insert_stmt = tx
        .prepare(
            "INSERT INTO package_resolution_health \
             (package_id, resolution_rate, resolved_refs, unresolved_refs, \
              low_conf_edges, trust_tier, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%s','now'))",
        )
        .context("pkg_health: prepare insert")?;

    for pid in package_ids {
        let edges: u64 = edges_stmt
            .query_row([pid], |r| r.get(0))
            .unwrap_or(0);
        let unresolved: u64 = unresolved_stmt
            .query_row([pid], |r| r.get(0))
            .unwrap_or(0);
        let low_conf: u64 = low_conf_stmt
            .query_row(rusqlite::params![pid, low_conf_threshold], |r| r.get(0))
            .unwrap_or(0);

        let total = edges + unresolved;
        let rate = if total > 0 {
            (edges as f64 / total as f64) * 100.0
        } else {
            100.0
        };
        let rate = (rate * 10.0).round() / 10.0;
        let low_conf_ratio = if edges > 0 {
            low_conf as f64 / edges as f64
        } else {
            0.0
        };
        let trust_tier = if rate >= 99.0 && low_conf_ratio < 0.05 {
            TrustTier::Trusted
        } else if rate >= 95.0 && low_conf_ratio < 0.15 {
            TrustTier::Review
        } else {
            TrustTier::Unsafe
        };

        insert_stmt
            .execute(rusqlite::params![
                pid,
                rate,
                edges as i64,
                unresolved as i64,
                low_conf as i64,
                trust_tier.as_str(),
            ])
            .context("pkg_health: insert row")?;
    }
    drop(insert_stmt);
    tx.commit().context("pkg_health: commit")?;
    Ok(())
}

/// Load per-package resolution rates as a multiplier in [0.5, 1.0]. A
/// package with resolution_rate = 100% returns 1.0; one with 95% returns
/// 0.95; below ~50% it floors at 0.5 so a single struggling package's
/// confidence dampening stays bounded. Missing packages return 1.0
/// (no penalty applied).
fn load_package_health_multipliers(
    conn: &rusqlite::Connection,
) -> QueryResult<std::collections::HashMap<i64, f64>> {
    let mut map = std::collections::HashMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT package_id, resolution_rate \
             FROM package_resolution_health",
        )
        .context("pkg_health: load multipliers")?;
    for row in stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))?
        .flatten()
    {
        let (pid, rate) = row;
        let mult = (rate / 100.0).max(0.5).min(1.0);
        map.insert(pid, mult);
    }
    Ok(map)
}

/// Build a map of `file_id → package_id` for fast per-symbol lookup in
/// `find_dead_code`. Files with no package_id are omitted.
fn load_file_to_package_map(
    conn: &rusqlite::Connection,
) -> QueryResult<std::collections::HashMap<i64, i64>> {
    let mut map = std::collections::HashMap::new();
    let mut stmt = conn
        .prepare("SELECT id, package_id FROM files WHERE package_id IS NOT NULL")
        .context("pkg_health: file→pkg map")?;
    for row in stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
        .flatten()
    {
        map.insert(row.0, row.1);
    }
    Ok(map)
}

/// Build a map of unresolved ref target names → count.
///
/// `scope_file_ids = None` counts every unresolved ref in the project
/// (original behavior). `Some(ids)` restricts the count to refs whose
/// **source symbol** lives in one of those files — so a monorepo with
/// two packages each containing a `handleClick` symbol won't keep
/// `apps/web`'s dead `handleClick` alive because `apps/api` failed to
/// resolve a different `handleClick`.
fn build_unresolved_name_counts(
    conn: &rusqlite::Connection,
    scope_file_ids: Option<&std::collections::HashSet<i64>>,
) -> QueryResult<std::collections::HashMap<String, u32>> {
    let mut map = std::collections::HashMap::new();
    let sql = match scope_file_ids {
        None => "SELECT target_name, COUNT(*) FROM unresolved_refs GROUP BY target_name"
            .to_string(),
        Some(ids) if ids.is_empty() => return Ok(map),
        Some(ids) => format!(
            "SELECT u.target_name, COUNT(*)
             FROM unresolved_refs u
             JOIN symbols s ON s.id = u.source_id
             WHERE s.file_id IN ({})
             GROUP BY u.target_name",
            ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",")
        ),
    };
    let mut stmt = conn
        .prepare(&sql)
        .context("unresolved_names: prepare")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))
        .context("unresolved_names: query")?;
    for row in rows.flatten() {
        map.insert(row.0, row.1);
    }
    Ok(map)
}

/// Resolve a scope string to the set of file_ids it covers.
///
/// Tried, in order:
///   1. Path prefix: files whose `path` starts with the scope literal.
///   2. Package match: files belonging to any package whose `declared_name`,
///      folder `name`, or `path` equals the scope.
///
/// Returns `None` when no scope was provided (whole-project queries),
/// `Some(empty)` when a scope was provided but matched nothing (callers
/// should treat that as "no candidates"), or `Some(non-empty)` with the
/// matching file ids.
fn resolve_scope_file_ids(
    conn: &rusqlite::Connection,
    scope: Option<&str>,
) -> QueryResult<Option<std::collections::HashSet<i64>>> {
    let Some(scope) = scope else { return Ok(None) };
    let mut file_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();

    let prefix = format!("{scope}%");
    let mut path_stmt = conn
        .prepare("SELECT id FROM files WHERE path LIKE ?1")
        .context("resolve_scope: file paths")?;
    for fid in path_stmt
        .query_map([&prefix], |r| r.get::<_, i64>(0))?
        .flatten()
    {
        file_ids.insert(fid);
    }

    let mut pkg_stmt = conn
        .prepare(
            "SELECT id FROM packages
             WHERE declared_name = ?1 OR name = ?1 OR path = ?1",
        )
        .context("resolve_scope: packages")?;
    let package_ids: Vec<i64> = pkg_stmt
        .query_map([scope], |r| r.get::<_, i64>(0))?
        .flatten()
        .collect();

    if !package_ids.is_empty() {
        let csv = package_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT id FROM files WHERE package_id IN ({csv})");
        let mut fstmt = conn.prepare(&sql).context("resolve_scope: pkg files")?;
        for fid in fstmt
            .query_map([], |r| r.get::<_, i64>(0))?
            .flatten()
        {
            file_ids.insert(fid);
        }
    }

    Ok(Some(file_ids))
}

/// Names that are too generic to use for unresolved ref matching.
/// A symbol named `value` having unresolved refs to `value` elsewhere in the
/// project is almost certainly a coincidence, not a real reference.
fn is_generic_name(name: &str) -> bool {
    matches!(
        name,
        "value" | "data" | "result" | "error" | "key" | "name" | "id" | "type"
            | "index" | "count" | "size" | "length" | "state" | "status"
            | "config" | "options" | "params" | "args" | "context" | "request"
            | "response" | "item" | "items" | "list" | "map" | "set"
            | "input" | "output" | "source" | "target" | "path" | "url"
            | "text" | "message" | "label" | "title" | "description"
            | "callback" | "handler" | "listener" | "observer"
            | "create" | "update" | "delete" | "get" | "add" | "remove"
            | "start" | "stop" | "open" | "close" | "read" | "write"
            | "load" | "save" | "init" | "reset" | "clear" | "build" | "run"
            | "apply" | "call" | "invoke" | "execute" | "process" | "handle"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "dead_code_tests.rs"]
mod tests;
