// =============================================================================
// query/entry_points.rs — Entry-point registry (reachability-based dead-code L1)
//
// Materializes the `entry_points` SQL table from a fixed set of contributors:
//
//   • main             — `main()`, `Main()`, `Program.Main`
//   • routes           — symbols in the `routes` table
//   • flow_edges       — event handlers and DI bindings (from `flow_edges`)
//   • exported_api     — public symbols in packages whose manifest declares
//                        a name (`Cargo.toml [package].name`, npm `name`, …)
//                        AND whose `packages.is_publishable` is 1
//   • lifecycle        — framework lifecycle method names
//   • test             — test functions (matched by name pattern or test path)
//   • language plugins — `LanguagePlugin::entry_points` (returns empty in v1)
//
// The dead-code BFS treats every distinct `symbol_id` here as a reachability
// root. `rebuild_entry_points` is idempotent — it clears the table and re-runs
// every contributor; safe to call lazily from query paths until Phase 3 moves
// the call into `finalize_resolution`.
// =============================================================================

use crate::db::Database;
use crate::query::QueryResult;
use crate::types::EntryPointRow;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Public types (moved from query/dead_code.rs; re-exported there for compat)
// ---------------------------------------------------------------------------

/// Why a symbol was classified as an entry point (and thus excluded from
/// dead-code reporting).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryPointKind {
    Main,
    RouteHandler,
    EventHandler,
    TestFunction,
    ExportedApi,
    LifecycleHook,
    DiRegistered,
    /// User-declared root via `.bw/roots.json`. Escape valve for genuinely
    /// dynamic dispatch that no static contributor can resolve.
    UserDeclared,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryPointsReport {
    pub total: u32,
    pub entry_points: Vec<EntryPoint>,
}

// ---------------------------------------------------------------------------
// Kind tag strings — the values written into `entry_points.kind`.
// Keep in sync with EntryPointKind::from_tag below.
// ---------------------------------------------------------------------------

pub const KIND_MAIN: &str = "main";
pub const KIND_ROUTE: &str = "route";
pub const KIND_EVENT: &str = "event";
pub const KIND_DI: &str = "di";
pub const KIND_EXPORTED: &str = "exported";
pub const KIND_LIFECYCLE: &str = "lifecycle";
pub const KIND_USER: &str = "user";
pub const KIND_TEST: &str = "test";

// ---------------------------------------------------------------------------
// Rebuild + load
// ---------------------------------------------------------------------------

/// Rebuild the `entry_points` table from all contributors.
///
/// Clears existing rows, runs every contributor, then bulk-inserts. Lazy
/// callers (`find_dead_code`, `find_entry_points`) invoke this on each
/// query; once Phase 3 moves the call into `finalize_resolution` the
/// per-query cost goes away.
pub fn rebuild_entry_points(db: &Database) -> QueryResult<()> {
    let conn = db.conn();
    conn.execute("DELETE FROM entry_points", [])
        .context("entry_points: clear table")?;

    let mut rows: Vec<EntryPointRow> = Vec::new();
    contribute_main(conn, &mut rows)?;
    contribute_routes(conn, &mut rows)?;
    contribute_flow_edges(conn, &mut rows)?;
    contribute_exported_api(conn, &mut rows)?;
    contribute_lifecycle(conn, &mut rows)?;
    contribute_test_functions(conn, &mut rows)?;
    contribute_user_roots(db, &mut rows)?;
    // Per-language plugin entry-points are wired in Phase 2.5+; today every
    // plugin's default returns empty so the loop is a no-op.

    if rows.is_empty() {
        return Ok(());
    }

    let mut stmt = conn
        .prepare(
            "INSERT OR IGNORE INTO entry_points \
             (symbol_id, kind, source, confidence) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .context("entry_points: prepare insert")?;
    for row in &rows {
        stmt.execute(rusqlite::params![
            row.symbol_id,
            row.kind,
            row.source,
            row.confidence,
        ])
        .context("entry_points: insert row")?;
    }
    Ok(())
}

/// Load entry-point symbol IDs used to *exclude* candidates from dead-code
/// detection. Preserves the historical scope of `collect_entry_point_ids`:
/// includes main / routes / flow_edges / exported_api / lifecycle but NOT
/// the test-function contributor — test files are excluded separately via
/// `is_test_file` so a `test_foo` declared in a non-test file stays
/// in-scope for dead-code analysis.
pub fn load_entry_point_ids_for_exclusion(db: &Database) -> QueryResult<HashSet<i64>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT symbol_id FROM entry_points \
             WHERE kind != ?1",
        )
        .context("entry_points: load exclusion ids")?;
    let mut out = HashSet::new();
    for row in stmt
        .query_map([KIND_TEST], |r| r.get::<_, i64>(0))?
        .flatten()
    {
        out.insert(row);
    }
    Ok(out)
}

/// Build the user-facing `EntryPointsReport`. Preserves the historical
/// scope of `find_entry_points`: includes main / routes / flow_edges /
/// exported_api / test functions, but NOT lifecycle hooks — that
/// asymmetry with the exclusion set is a quirk of the v0 report and
/// stays unchanged in this port. Phase 3+ may unify both views.
pub fn find_entry_points(db: &Database) -> QueryResult<EntryPointsReport> {
    let _timer = db.timer("entry_points");
    rebuild_entry_points(db)?;

    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT s.id, s.name, s.qualified_name, s.kind, f.path, s.line, ep.kind \
             FROM entry_points ep \
             JOIN symbols s ON s.id = ep.symbol_id \
             JOIN files   f ON f.id = s.file_id \
             WHERE ep.kind != ?1 \
             ORDER BY ep.kind, f.path, s.line",
        )
        .context("entry_points: build report query")?;

    let mut seen = HashSet::new();
    let mut entries: Vec<EntryPoint> = Vec::new();
    let rows = stmt
        .query_map([KIND_LIFECYCLE], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, u32>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .context("entry_points: execute report query")?;

    for row in rows.flatten() {
        let (id, name, qname, kind, path, line, ep_kind_tag) = row;
        if !seen.insert(id) {
            continue;
        }
        let entry_kind = match ep_kind_tag.as_str() {
            KIND_MAIN => EntryPointKind::Main,
            KIND_ROUTE => EntryPointKind::RouteHandler,
            KIND_EVENT => EntryPointKind::EventHandler,
            KIND_DI => EntryPointKind::DiRegistered,
            KIND_EXPORTED => EntryPointKind::ExportedApi,
            KIND_TEST => EntryPointKind::TestFunction,
            KIND_USER => EntryPointKind::UserDeclared,
            // Lifecycle is filtered above; an unknown tag falls back to
            // Main rather than panicking to keep the report robust to
            // future contributor additions.
            _ => EntryPointKind::Main,
        };
        entries.push(EntryPoint {
            symbol_id: id,
            name,
            qualified_name: qname,
            kind,
            file_path: path,
            line,
            entry_kind,
        });
    }

    let total = entries.len() as u32;
    Ok(EntryPointsReport {
        total,
        entry_points: entries,
    })
}

// ---------------------------------------------------------------------------
// Contributors
// ---------------------------------------------------------------------------

/// `main` / `Main` / `Program.Main` functions and methods.
fn contribute_main(conn: &rusqlite::Connection, out: &mut Vec<EntryPointRow>) -> QueryResult<()> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM symbols \
             WHERE name IN ('main', 'Main', 'Program.Main') \
               AND kind IN ('function', 'method') \
               AND origin = 'internal'",
        )
        .context("entry_points: prepare main")?;
    for id in stmt.query_map([], |r| r.get::<_, i64>(0))?.flatten() {
        out.push(EntryPointRow {
            symbol_id: id,
            kind: KIND_MAIN,
            source: "main-contributor",
            confidence: 1.0,
        });
    }
    Ok(())
}

/// Symbols recorded in the `routes` table.
fn contribute_routes(conn: &rusqlite::Connection, out: &mut Vec<EntryPointRow>) -> QueryResult<()> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT symbol_id FROM routes WHERE symbol_id IS NOT NULL")
        .context("entry_points: prepare routes")?;
    for id in stmt.query_map([], |r| r.get::<_, i64>(0))?.flatten() {
        out.push(EntryPointRow {
            symbol_id: id,
            kind: KIND_ROUTE,
            source: "routes-contributor",
            confidence: 1.0,
        });
    }
    Ok(())
}

/// Event handlers + DI bindings recorded in `flow_edges`. Joins on the
/// ±2-line window — the same brittle heuristic as the original
/// `collect_entry_point_ids`; replaced in Phase 6 by direct
/// `reflection_root` edges from connectors.
fn contribute_flow_edges(
    conn: &rusqlite::Connection,
    out: &mut Vec<EntryPointRow>,
) -> QueryResult<()> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT s.id, fe.edge_type \
             FROM flow_edges fe \
             JOIN files f ON f.id = fe.target_file_id \
             JOIN symbols s ON s.file_id = f.id \
               AND s.line BETWEEN fe.target_line - 2 AND fe.target_line + 2 \
             WHERE fe.edge_type IN ('event_handler', 'di_binding')",
        )
        .context("entry_points: prepare flow_edges")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .context("entry_points: execute flow_edges")?;
    for row in rows.flatten() {
        let (id, edge_type) = row;
        let kind = if edge_type == "di_binding" {
            KIND_DI
        } else {
            KIND_EVENT
        };
        out.push(EntryPointRow {
            symbol_id: id,
            kind,
            source: "flow-edges-contributor",
            confidence: 0.8,
        });
    }
    Ok(())
}

/// Public symbols inside packages that declare a manifest name AND are
/// marked publishable. The `is_publishable` gate is the Phase-5 hook
/// that lets a workspace package opt out of being treated as external
/// library API; today every existing package has `is_publishable = 1`
/// from the migration default so behavior is preserved.
fn contribute_exported_api(
    conn: &rusqlite::Connection,
    out: &mut Vec<EntryPointRow>,
) -> QueryResult<()> {
    // Two strata in a single UNION query:
    //
    //   • Directly-exported symbols: anything with `visibility='public'`
    //     in a publishable-manifest package. Catches top-level
    //     `export function/class/...` declarations across every language.
    //
    //   • Methods of exported classes: methods/constructors whose
    //     `scope_path` points at one of the directly-exported types.
    //     Closes the "class entry point doesn't anchor its methods" gap
    //     surfaced after Phase 4 — `export class Foo { bar() {} }` makes
    //     `bar` externally callable even when its own visibility tag is
    //     NULL (the TS extractor doesn't mark class members public in
    //     the absence of an explicit modifier). Explicitly `private` /
    //     `protected` members are filtered out so they don't auto-root.
    let mut stmt = conn
        .prepare(
            "SELECT s.id FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             JOIN packages p ON p.id = f.package_id \
             WHERE s.visibility = 'public' \
               AND s.origin = 'internal' \
               AND p.declared_name IS NOT NULL \
               AND p.declared_name != '' \
               AND p.is_publishable = 1 \
               AND s.kind IN ('function','method','class','struct', \
                              'interface','enum','type_alias','trait', \
                              'protocol','module') \
             UNION \
             SELECT m.id FROM symbols m \
             JOIN symbols cls ON cls.qualified_name = m.scope_path \
             JOIN files cls_f ON cls_f.id = cls.file_id \
             JOIN packages cls_p ON cls_p.id = cls_f.package_id \
             WHERE m.kind IN ('method','function','constructor') \
               AND m.origin = 'internal' \
               AND (m.visibility IS NULL OR m.visibility NOT IN ('private','protected')) \
               AND cls.visibility = 'public' \
               AND cls.origin = 'internal' \
               AND cls.kind IN ('class','struct','interface','enum','trait','protocol') \
               AND cls_p.declared_name IS NOT NULL \
               AND cls_p.declared_name != '' \
               AND cls_p.is_publishable = 1",
        )
        .context("entry_points: prepare exported_api")?;
    for id in stmt.query_map([], |r| r.get::<_, i64>(0))?.flatten() {
        out.push(EntryPointRow {
            symbol_id: id,
            kind: KIND_EXPORTED,
            source: "manifest-exported-api",
            confidence: 0.9,
        });
    }
    Ok(())
}

/// Common framework lifecycle hooks. This is a hardcoded name list —
/// kept for behavior parity with `collect_entry_point_ids`, but slated
/// for removal in Phase 6 once per-language plugins emit lifecycle
/// roots through their own `entry_points` impls.
fn contribute_lifecycle(
    conn: &rusqlite::Connection,
    out: &mut Vec<EntryPointRow>,
) -> QueryResult<()> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM symbols \
             WHERE name IN ( \
                 'OnInit', 'OnDestroy', 'OnChanges', 'AfterViewInit', \
                 'ngOnInit', 'ngOnDestroy', 'ngOnChanges', 'ngAfterViewInit', \
                 'componentDidMount', 'componentWillUnmount', 'componentDidUpdate', \
                 'connectedCallback', 'disconnectedCallback', \
                 'Configure', 'ConfigureServices', \
                 'setUp', 'tearDown', 'setUpAll', 'tearDownAll', \
                 'initState', 'dispose', 'build', \
                 'setup', 'created', 'mounted', 'unmounted', 'beforeDestroy' \
             ) \
             AND kind IN ('function', 'method') \
             AND origin = 'internal'",
        )
        .context("entry_points: prepare lifecycle")?;
    for id in stmt.query_map([], |r| r.get::<_, i64>(0))?.flatten() {
        out.push(EntryPointRow {
            symbol_id: id,
            kind: KIND_LIFECYCLE,
            source: "lifecycle-contributor",
            confidence: 0.7,
        });
    }
    Ok(())
}

/// Test functions matched by name pattern or file path. Preserves the
/// SQL from the original `find_entry_points` step 5 so the report's
/// test-coverage stays identical.
fn contribute_test_functions(
    conn: &rusqlite::Connection,
    out: &mut Vec<EntryPointRow>,
) -> QueryResult<()> {
    let mut stmt = conn
        .prepare(
            "SELECT s.id FROM symbols s \
             JOIN files f ON f.id = s.file_id \
             WHERE s.kind IN ('function', 'method', 'test') \
               AND s.origin = 'internal' \
               AND (s.name LIKE 'test_%' \
                 OR s.name LIKE 'Test%' \
                 OR s.kind = 'test' \
                 OR f.path LIKE '%/test/%' \
                 OR f.path LIKE '%/tests/%' \
                 OR f.path LIKE '%/__tests__/%' \
                 OR f.path LIKE '%.test.%' \
                 OR f.path LIKE '%.spec.%' \
                 OR f.path LIKE '%_test.%')",
        )
        .context("entry_points: prepare test_functions")?;
    for id in stmt.query_map([], |r| r.get::<_, i64>(0))?.flatten() {
        out.push(EntryPointRow {
            symbol_id: id,
            kind: KIND_TEST,
            source: "test-contributor",
            confidence: 1.0,
        });
    }
    Ok(())
}

/// User-declared reachability roots from `<project_root>/.bw/roots.json`.
/// The escape valve for genuinely dynamic dispatch the static analyzer
/// can't see (eval, framework auto-discovery scanning strings, custom
/// reflection registries). Three pattern types:
///
///   • `qnames`: exact `qualified_name` matches.
///   • `names`: glob-on-`name` (one segment, `*` and `?` wildcards via
///              SQL `LIKE` translation).
///   • `globs`: file-path glob (`*` → `%`, `?` → `_`); every symbol in
///              a matching file becomes a root.
///
/// Skipped silently when the project has no DB path on disk (in-memory
/// test DBs) or the file doesn't exist. Malformed JSON logs a warning
/// rather than failing the whole query — the rest of the contributor
/// model keeps running.
fn contribute_user_roots(db: &Database, out: &mut Vec<EntryPointRow>) -> QueryResult<()> {
    let Some(db_path) = db.path.as_ref() else {
        return Ok(());
    };
    // db path is `<project_root>/.bearwisdom/index.db`; project_root is two
    // parents up. Walking through `.parent().parent()` keeps the resolver
    // honest if a future caller opens a DB from somewhere unusual.
    let Some(project_root) = db_path.parent().and_then(|p| p.parent()) else {
        return Ok(());
    };
    let roots_file = project_root.join(".bw").join("roots.json");
    if !roots_file.exists() {
        return Ok(());
    }

    let content = match std::fs::read_to_string(&roots_file) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("entry_points: failed to read {:?}: {e}", roots_file);
            return Ok(());
        }
    };
    let cfg: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("entry_points: {:?} is not valid JSON: {e}", roots_file);
            return Ok(());
        }
    };

    let conn = db.conn();

    // qnames — exact qualified_name match.
    if let Some(arr) = cfg.get("qnames").and_then(|v| v.as_array()) {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM symbols \
                 WHERE qualified_name = ?1 AND origin = 'internal'",
            )
            .context("entry_points: prepare user qname query")?;
        for entry in arr {
            let Some(q) = entry.as_str() else {
                continue;
            };
            for id in stmt.query_map([q], |r| r.get::<_, i64>(0))?.flatten() {
                out.push(EntryPointRow {
                    symbol_id: id,
                    kind: KIND_USER,
                    source: "user-roots-json",
                    confidence: 1.0,
                });
            }
        }
    }

    // names — glob on `name`. Translate `*`/`?` to SQL `LIKE` wildcards.
    if let Some(arr) = cfg.get("names").and_then(|v| v.as_array()) {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM symbols \
                 WHERE name LIKE ?1 AND origin = 'internal'",
            )
            .context("entry_points: prepare user name query")?;
        for entry in arr {
            let Some(pat) = entry.as_str() else {
                continue;
            };
            let sql_pat = glob_to_sql_like(pat);
            for id in stmt
                .query_map([sql_pat.as_str()], |r| r.get::<_, i64>(0))?
                .flatten()
            {
                out.push(EntryPointRow {
                    symbol_id: id,
                    kind: KIND_USER,
                    source: "user-roots-json",
                    confidence: 1.0,
                });
            }
        }
    }

    // globs — every internal symbol in any file whose path matches.
    if let Some(arr) = cfg.get("globs").and_then(|v| v.as_array()) {
        let mut stmt = conn
            .prepare(
                "SELECT s.id FROM symbols s \
                 JOIN files f ON f.id = s.file_id \
                 WHERE f.path LIKE ?1 AND s.origin = 'internal'",
            )
            .context("entry_points: prepare user glob query")?;
        for entry in arr {
            let Some(pat) = entry.as_str() else {
                continue;
            };
            let sql_pat = glob_to_sql_like(pat);
            for id in stmt
                .query_map([sql_pat.as_str()], |r| r.get::<_, i64>(0))?
                .flatten()
            {
                out.push(EntryPointRow {
                    symbol_id: id,
                    kind: KIND_USER,
                    source: "user-roots-json",
                    confidence: 1.0,
                });
            }
        }
    }

    Ok(())
}

/// Translate a `*`/`?` glob to SQL `LIKE` wildcards. SQLite's LIKE has no
/// recursive wildcard (`**`); a single `*` already spans path separators
/// since it expands to `%`, so `src/**/*.ts` and `src/*.ts` behave the
/// same way. Literal `%` and `_` characters in user input map to the same
/// SQL meta chars — rare enough in real symbol names and file paths that
/// we don't introduce an ESCAPE clause to handle them.
fn glob_to_sql_like(pat: &str) -> String {
    let mut out = String::with_capacity(pat.len());
    for ch in pat.chars() {
        match ch {
            '*' => out.push('%'),
            '?' => out.push('_'),
            _ => out.push(ch),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "entry_points_tests.rs"]
mod tests;
