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
}

/// Why a symbol was classified as an entry point (and thus excluded).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryPointKind {
    /// `main()`, `Main()`, program entry.
    Main,
    /// HTTP route handler (in `routes` table).
    RouteHandler,
    /// Event handler, message queue subscriber (in `flow_edges`).
    EventHandler,
    /// Test function (in a test file or named test_*).
    TestFunction,
    /// Public symbol in a library package.
    ExportedApi,
    /// Framework lifecycle hook.
    LifecycleHook,
    /// DI-registered service (referenced in `flow_edges` as di_binding).
    DiRegistered,
}

/// An identified entry point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryPoint {
    pub symbol_id: i64,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file_path: String,
    pub line: u32,
    pub entry_kind: EntryPointKind,
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

/// Full entry points report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryPointsReport {
    pub total: u32,
    pub entry_points: Vec<EntryPoint>,
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
    let resolution_health = compute_resolution_health(conn)?;

    // --- Build unresolved ref targets for cross-referencing ---
    // Maps (target_name) → count of unresolved refs with that target.
    // We also build a set of (target_name, file_id) pairs for same-file matching.
    let unresolved_names = build_unresolved_name_counts(conn)?;

    // Collect entry point symbol IDs for exclusion.
    let entry_point_ids = collect_entry_point_ids(conn)?;
    let test_file_ids = collect_test_file_ids(conn)?;

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

    // Build the SQL query based on options.
    let mut sql = String::from(
        "SELECT s.id, s.name, s.qualified_name, s.kind, s.visibility,
                f.path, s.line, s.incoming_edge_count, f.id as file_id
         FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.incoming_edge_count = 0
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
    if let Some(scope) = &options.scope {
        sql.push_str(&format!(
            " AND (f.path LIKE '{}%')",
            scope.replace('\'', "''")
        ));
    }

    sql.push_str(" ORDER BY f.path, s.line");

    let mut stmt = conn.prepare(&sql).context("dead_code: prepare query")?;

    // Bind kind parameters.
    let params: Vec<Box<dyn rusqlite::types::ToSql>> = check_kinds
        .iter()
        .map(|k| Box::new(k.to_string()) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(|p| p.as_ref()).collect();

    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,       // id
                row.get::<_, String>(1)?,     // name
                row.get::<_, String>(2)?,     // qualified_name
                row.get::<_, String>(3)?,     // kind
                row.get::<_, Option<String>>(4)?, // visibility
                row.get::<_, String>(5)?,     // path
                row.get::<_, u32>(6)?,        // line
                row.get::<_, i64>(7)?,        // incoming_edge_count
                row.get::<_, i64>(8)?,        // file_id
            ))
        })
        .context("dead_code: execute query")?;

    let mut candidates = Vec::new();
    let mut total_checked: u32 = 0;
    let mut entry_points_excluded: u32 = 0;
    let mut test_symbols_excluded: u32 = 0;

    for row in rows {
        let (id, name, qname, kind, visibility, path, line, _incoming, file_id) =
            row.context("dead_code: read row")?;

        total_checked += 1;

        // Exclude entry points.
        if entry_point_ids.contains(&id) {
            entry_points_excluded += 1;
            continue;
        }

        // Exclude test files.
        if !options.include_tests && test_file_ids.contains(&file_id) {
            test_symbols_excluded += 1;
            continue;
        }

        // Skip common noise symbols.
        if is_noise_symbol(&name, &kind) {
            continue;
        }

        // Compute base confidence from visibility.
        let mut confidence = match visibility.as_deref() {
            Some("private") | None => 1.0,
            Some("internal") => 0.9,
            Some("public") => 0.7,
            _ => 0.8,
        };

        // Cross-reference against unresolved refs.
        // If this symbol's name or qualified name appears as an unresolved target,
        // it may still be referenced — lower confidence and flag it.
        let unresolved_match_count = unresolved_names.get(qname.as_str())
            .or_else(|| {
                // Only match by simple name if it's not a generic name
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
            // Halve confidence — this symbol might be alive
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
        });

        if candidates.len() >= options.max_results {
            break;
        }
    }

    // Also find symbols with ONLY low-confidence edges (if room remains).
    if candidates.len() < options.max_results {
        let remaining = options.max_results - candidates.len();
        let existing_ids: std::collections::HashSet<i64> =
            candidates.iter().map(|c| c.symbol_id).collect();

        let low_conf = find_low_confidence_only(
            conn,
            &entry_point_ids,
            &test_file_ids,
            &options,
            remaining,
        )?;

        for entry in low_conf {
            if !existing_ids.contains(&entry.symbol_id) {
                candidates.push(entry);
            }
        }
    }

    // Resolution-gate trust tier: in `Unsafe`, suppress the high-confidence
    // signal so callers can't act on the report as if it were ground truth.
    // Cap every candidate at 0.5 — they're informational only at that
    // resolution level. See research/.../01-resolution-gate-plan.md §5.
    if resolution_health.trust_tier == TrustTier::Unsafe {
        for c in &mut candidates {
            if c.confidence > 0.5 {
                c.confidence = 0.5;
            }
        }
    }

    // Sort: non-potentially-referenced first, then by confidence descending.
    candidates.sort_by(|a, b| {
        a.potentially_referenced
            .cmp(&b.potentially_referenced)
            .then_with(|| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let potentially_referenced_count = candidates
        .iter()
        .filter(|c| c.potentially_referenced)
        .count() as u32;

    Ok(DeadCodeReport {
        total_symbols_checked: total_checked,
        dead_candidates: candidates,
        entry_points_excluded,
        test_symbols_excluded,
        potentially_referenced_count,
        resolution_health,
    })
}

// ---------------------------------------------------------------------------
// Entry point discovery
// ---------------------------------------------------------------------------

/// Find all entry points in the project.
pub fn find_entry_points(db: &Database) -> QueryResult<EntryPointsReport> {
    let _timer = db.timer("entry_points");
    let conn = db.conn();
    let mut entry_points = Vec::new();

    // 1. Main functions
    {
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.name, s.qualified_name, s.kind, f.path, s.line
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE s.name IN ('main', 'Main', 'Program.Main')
                   AND s.kind IN ('function', 'method')
                   AND s.origin = 'internal'",
            )
            .context("entry_points: prepare main query")?;

        let rows = stmt
            .query_map([], |row| {
                Ok(EntryPoint {
                    symbol_id: row.get(0)?,
                    name: row.get(1)?,
                    qualified_name: row.get(2)?,
                    kind: row.get(3)?,
                    file_path: row.get(4)?,
                    line: row.get(5)?,
                    entry_kind: EntryPointKind::Main,
                })
            })
            .context("entry_points: execute main query")?;

        for row in rows.flatten() {
            entry_points.push(row);
        }
    }

    // 2. Route handlers
    {
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.name, s.qualified_name, s.kind, f.path, s.line
                 FROM routes r
                 JOIN symbols s ON s.id = r.symbol_id
                 JOIN files f ON f.id = s.file_id
                 WHERE r.symbol_id IS NOT NULL",
            )
            .context("entry_points: prepare route query")?;

        let rows = stmt
            .query_map([], |row| {
                Ok(EntryPoint {
                    symbol_id: row.get(0)?,
                    name: row.get(1)?,
                    qualified_name: row.get(2)?,
                    kind: row.get(3)?,
                    file_path: row.get(4)?,
                    line: row.get(5)?,
                    entry_kind: EntryPointKind::RouteHandler,
                })
            })
            .context("entry_points: execute route query")?;

        for row in rows.flatten() {
            entry_points.push(row);
        }
    }

    // 3. Event handlers / DI bindings (from flow_edges)
    {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT s.id, s.name, s.qualified_name, s.kind, f.path, s.line,
                        fe.edge_type
                 FROM flow_edges fe
                 JOIN files f ON f.id = fe.target_file_id
                 JOIN symbols s ON s.file_id = f.id
                   AND s.line BETWEEN fe.target_line - 2 AND fe.target_line + 2
                 WHERE fe.edge_type IN ('event_handler', 'di_binding')",
            )
            .context("entry_points: prepare event/DI query")?;

        let rows = stmt
            .query_map([], |row| {
                let edge_type: String = row.get(6)?;
                let entry_kind = if edge_type == "di_binding" {
                    EntryPointKind::DiRegistered
                } else {
                    EntryPointKind::EventHandler
                };
                Ok(EntryPoint {
                    symbol_id: row.get(0)?,
                    name: row.get(1)?,
                    qualified_name: row.get(2)?,
                    kind: row.get(3)?,
                    file_path: row.get(4)?,
                    line: row.get(5)?,
                    entry_kind,
                })
            })
            .context("entry_points: execute event/DI query")?;

        for row in rows.flatten() {
            entry_points.push(row);
        }
    }

    // 4. Test functions (in test files or named test_*)
    {
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.name, s.qualified_name, s.kind, f.path, s.line
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE s.kind IN ('function', 'method', 'test')
                   AND s.origin = 'internal'
                   AND (s.name LIKE 'test_%'
                     OR s.name LIKE 'Test%'
                     OR s.kind = 'test'
                     OR f.path LIKE '%/test/%'
                     OR f.path LIKE '%/tests/%'
                     OR f.path LIKE '%/__tests__/%'
                     OR f.path LIKE '%.test.%'
                     OR f.path LIKE '%.spec.%'
                     OR f.path LIKE '%_test.%')",
            )
            .context("entry_points: prepare test query")?;

        let rows = stmt
            .query_map([], |row| {
                Ok(EntryPoint {
                    symbol_id: row.get(0)?,
                    name: row.get(1)?,
                    qualified_name: row.get(2)?,
                    kind: row.get(3)?,
                    file_path: row.get(4)?,
                    line: row.get(5)?,
                    entry_kind: EntryPointKind::TestFunction,
                })
            })
            .context("entry_points: execute test query")?;

        for row in rows.flatten() {
            entry_points.push(row);
        }
    }

    // Deduplicate by symbol_id (a symbol can match multiple categories).
    let mut seen = std::collections::HashSet::new();
    entry_points.retain(|ep| seen.insert(ep.symbol_id));

    let total = entry_points.len() as u32;
    Ok(EntryPointsReport {
        total,
        entry_points,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Collect all symbol IDs that are entry points (for exclusion from dead code).
fn collect_entry_point_ids(
    conn: &rusqlite::Connection,
) -> QueryResult<std::collections::HashSet<i64>> {
    let mut ids = std::collections::HashSet::new();

    // Main functions
    let mut stmt = conn
        .prepare(
            "SELECT id FROM symbols
             WHERE name IN ('main', 'Main', 'Program.Main')
               AND kind IN ('function', 'method')
               AND origin = 'internal'",
        )
        .context("entry_point_ids: main")?;
    for row in stmt.query_map([], |r| r.get::<_, i64>(0))?.flatten() {
        ids.insert(row);
    }

    // Route handlers
    let mut stmt = conn
        .prepare("SELECT DISTINCT symbol_id FROM routes WHERE symbol_id IS NOT NULL")
        .context("entry_point_ids: routes")?;
    for row in stmt.query_map([], |r| r.get::<_, i64>(0))?.flatten() {
        ids.insert(row);
    }

    // Flow edge targets (event handlers, DI bindings)
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT s.id
             FROM flow_edges fe
             JOIN files f ON f.id = fe.target_file_id
             JOIN symbols s ON s.file_id = f.id
               AND s.line BETWEEN fe.target_line - 2 AND fe.target_line + 2
             WHERE fe.edge_type IN ('event_handler', 'di_binding')",
        )
        .context("entry_point_ids: flow_edges")?;
    for row in stmt.query_map([], |r| r.get::<_, i64>(0))?.flatten() {
        ids.insert(row);
    }

    // Lifecycle hooks — common framework patterns
    let mut stmt = conn
        .prepare(
            "SELECT id FROM symbols
             WHERE name IN (
                 'OnInit', 'OnDestroy', 'OnChanges', 'AfterViewInit',
                 'ngOnInit', 'ngOnDestroy', 'ngOnChanges', 'ngAfterViewInit',
                 'componentDidMount', 'componentWillUnmount', 'componentDidUpdate',
                 'connectedCallback', 'disconnectedCallback',
                 'Configure', 'ConfigureServices',
                 'setUp', 'tearDown', 'setUpAll', 'tearDownAll',
                 'initState', 'dispose', 'build',
                 'setup', 'created', 'mounted', 'unmounted', 'beforeDestroy'
             )
             AND kind IN ('function', 'method')
             AND origin = 'internal'",
        )
        .context("entry_point_ids: lifecycle")?;
    for row in stmt.query_map([], |r| r.get::<_, i64>(0))?.flatten() {
        ids.insert(row);
    }

    Ok(ids)
}

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

/// Find symbols whose ONLY incoming edges are low-confidence (<0.7).
fn find_low_confidence_only(
    conn: &rusqlite::Connection,
    entry_point_ids: &std::collections::HashSet<i64>,
    test_file_ids: &std::collections::HashSet<i64>,
    options: &DeadCodeOptions,
    limit: usize,
) -> QueryResult<Vec<DeadCodeEntry>> {
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.name, s.qualified_name, s.kind, s.visibility,
                    f.path, s.line, s.incoming_edge_count, f.id
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE s.incoming_edge_count > 0
               AND s.origin = 'internal'
               AND s.kind IN ('function', 'method', 'class', 'struct', 'interface', 'enum')
               AND EXISTS (
                   SELECT 1 FROM edges e WHERE e.target_id = s.id
               )
               AND NOT EXISTS (
                   SELECT 1 FROM edges e
                   WHERE e.target_id = s.id AND e.confidence >= 0.7
               )
             ORDER BY s.incoming_edge_count ASC
             LIMIT ?1",
        )
        .context("low_confidence: prepare")?;

    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, u32>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
            ))
        })
        .context("low_confidence: execute")?;

    let mut results = Vec::new();
    for row in rows.flatten() {
        let (id, name, qname, kind, visibility, path, line, _incoming, file_id) = row;

        if entry_point_ids.contains(&id) {
            continue;
        }
        if !options.include_tests && test_file_ids.contains(&file_id) {
            continue;
        }
        if is_noise_symbol(&name, &kind) {
            continue;
        }

        results.push(DeadCodeEntry {
            symbol_id: id,
            name,
            qualified_name: qname,
            kind,
            visibility,
            file_path: path,
            line,
            confidence: 0.3,
            reason: DeadCodeReason::OnlyLowConfidenceEdges,
            potentially_referenced: false,
            unresolved_ref_matches: None,
        });
    }

    Ok(results)
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

/// Compute overall resolution health for the project.
///
/// Uses the same metric the resolution-gate plan defines:
/// `internal_edges / (internal_edges + internal_unresolved)` where both
/// sides are restricted to first-party (`origin = 'internal'`) source.
/// Doc-snippet refs are excluded via the same `CODE_REF_FILTER` the
/// `resolution_breakdown` query uses, so the dead-code health number
/// matches the gate metric exactly.
fn compute_resolution_health(
    conn: &rusqlite::Connection,
) -> QueryResult<ResolutionHealth> {
    let internal_edges: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let internal_unresolved_sql = format!(
        "SELECT COUNT(*)
         FROM unresolved_refs u
         JOIN symbols s ON s.id = u.source_id
         JOIN files   f ON f.id = s.file_id
         WHERE f.origin = 'internal' AND {filter}",
        filter = crate::query::stats::CODE_REF_FILTER
    );
    let internal_unresolved: u64 = conn
        .query_row(&internal_unresolved_sql, [], |row| row.get(0))
        .unwrap_or(0);

    let low_conf_threshold = crate::query::diagnostics::LOW_CONFIDENCE_THRESHOLD;
    let low_confidence_edges: u64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal' AND e.confidence < ?1",
            [low_conf_threshold],
            |row| row.get(0),
        )
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

/// Build a map of unresolved ref target names → count.
/// Used to cross-reference dead code candidates.
fn build_unresolved_name_counts(
    conn: &rusqlite::Connection,
) -> QueryResult<std::collections::HashMap<String, u32>> {
    let mut map = std::collections::HashMap::new();
    let mut stmt = conn
        .prepare("SELECT target_name, COUNT(*) FROM unresolved_refs GROUP BY target_name")
        .context("unresolved_names: prepare")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))
        .context("unresolved_names: query")?;
    for row in rows.flatten() {
        map.insert(row.0, row.1);
    }
    Ok(map)
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
mod tests {
    use super::*;
    use crate::db::Database;

    fn setup_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        // File
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/lib.rs', 'h1', 'rust', 0)",
            [],
        ).unwrap();
        let file_id = conn.last_insert_rowid();

        // Called function (has incoming edge)
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, incoming_edge_count)
             VALUES (?1, 'used_fn', 'mod::used_fn', 'function', 10, 0, 'public', 3)",
            [file_id],
        ).unwrap();

        // Uncalled private function (dead code candidate)
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, incoming_edge_count)
             VALUES (?1, 'dead_fn', 'mod::dead_fn', 'function', 20, 0, 'private', 0)",
            [file_id],
        ).unwrap();

        // Main function (entry point, should be excluded)
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, incoming_edge_count)
             VALUES (?1, 'main', 'main', 'function', 1, 0, NULL, 0)",
            [file_id],
        ).unwrap();

        // Test file — path must contain /tests/ or /test/ with a leading slash
        // to match is_test_file(), which checks for "/tests/" substring.
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/tests/test_lib.rs', 'h2', 'rust', 0)",
            [],
        ).unwrap();
        let test_file_id = conn.last_insert_rowid();

        // Test function (should be excluded)
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, incoming_edge_count)
             VALUES (?1, 'test_something', 'tests::test_something', 'function', 5, 0, NULL, 0)",
            [test_file_id],
        ).unwrap();

        db
    }

    #[test]
    fn dead_code_finds_uncalled_private_fn() {
        let db = setup_db();
        let report = find_dead_code(&db, &DeadCodeOptions::default()).unwrap();

        assert_eq!(report.dead_candidates.len(), 1);
        assert_eq!(report.dead_candidates[0].name, "dead_fn");
        assert_eq!(report.dead_candidates[0].confidence, 1.0);
    }

    #[test]
    fn dead_code_excludes_main() {
        let db = setup_db();
        let report = find_dead_code(&db, &DeadCodeOptions::default()).unwrap();

        assert!(report.entry_points_excluded > 0);
        assert!(!report
            .dead_candidates
            .iter()
            .any(|c| c.name == "main"));
    }

    #[test]
    fn dead_code_excludes_test_files() {
        let db = setup_db();
        let report = find_dead_code(&db, &DeadCodeOptions::default()).unwrap();

        assert!(report.test_symbols_excluded > 0);
        assert!(!report
            .dead_candidates
            .iter()
            .any(|c| c.name == "test_something"));
    }

    #[test]
    fn dead_code_includes_tests_when_asked() {
        let db = setup_db();
        let opts = DeadCodeOptions {
            include_tests: true,
            ..Default::default()
        };
        let report = find_dead_code(&db, &opts).unwrap();

        assert!(report
            .dead_candidates
            .iter()
            .any(|c| c.name == "test_something"));
    }

    #[test]
    fn dead_code_respects_visibility_filter() {
        let db = setup_db();

        let opts = DeadCodeOptions {
            visibility_filter: VisibilityFilter::PublicOnly,
            ..Default::default()
        };
        let report = find_dead_code(&db, &opts).unwrap();
        // dead_fn is private, should not appear
        assert!(!report
            .dead_candidates
            .iter()
            .any(|c| c.name == "dead_fn"));
    }

    #[test]
    fn entry_points_finds_main() {
        let db = setup_db();
        let report = find_entry_points(&db).unwrap();

        assert!(report
            .entry_points
            .iter()
            .any(|ep| ep.name == "main" && matches!(ep.entry_kind, EntryPointKind::Main)));
    }

    #[test]
    fn entry_points_finds_test_functions() {
        let db = setup_db();
        let report = find_entry_points(&db).unwrap();

        assert!(report.entry_points.iter().any(
            |ep| ep.name == "test_something"
                && matches!(ep.entry_kind, EntryPointKind::TestFunction)
        ));
    }

    #[test]
    fn empty_db_returns_empty_report() {
        let db = Database::open_in_memory().unwrap();
        let report = find_dead_code(&db, &DeadCodeOptions::default()).unwrap();
        assert!(report.dead_candidates.is_empty());
        assert_eq!(report.total_symbols_checked, 0);
    }
}
