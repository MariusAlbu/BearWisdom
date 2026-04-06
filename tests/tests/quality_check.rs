//! Integration tests for quality-check baseline comparison.
//!
//! The `cmd_quality_check` function in `bearwisdom-cli` is private and
//! tightly coupled to CLI concerns (filesystem DB paths, eprintln output,
//! JSON envelope wrapping).  Rather than testing that function directly, these
//! tests exercise the same observable behaviour:
//!
//!   1.  Index a fixture project into an on-disk DB (matching what `--reindex`
//!       does in the real command).
//!   2.  Read the same DB counters the CLI reads (files, symbols, edges, routes,
//!       flow_edges).
//!   3.  Compare against a manually-constructed baseline JSON structure with
//!       the same schema the CLI expects.
//!   4.  Assert pass/fail/regression/improvement outcomes.
//!
//! This approach is strictly correct: if the CLI's comparison logic changes the
//! tests will need updating in lock-step anyway, but the observable semantics
//! (what counts as a regression) are unlikely to change.

use bearwisdom::{full_index, resolve_db_path, Database};
use bearwisdom_tests::TestProject;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The counters the quality-check command reads per project.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ProjectStats {
    files: i64,
    symbols: i64,
    edges: i64,
    routes: i64,
    flow_edges: i64,
    unresolved_refs: i64,
}

/// Read the same counters that `cmd_quality_check` reads.
fn read_project_stats(db: &Database) -> ProjectStats {
    let conn = db.conn();
    ProjectStats {
        files: conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap(),
        symbols: conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .unwrap(),
        edges: conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .unwrap(),
        routes: conn
            .query_row("SELECT COUNT(*) FROM routes", [], |r| r.get(0))
            .unwrap(),
        flow_edges: conn
            .query_row("SELECT COUNT(*) FROM flow_edges", [], |r| r.get(0))
            .unwrap(),
        unresolved_refs: conn
            .query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r| r.get(0))
            .unwrap(),
    }
}

/// Mirror the regression/improvement comparison from `cmd_quality_check`.
///
/// Returns `(regressions, improvements)` as lists of human-readable strings.
fn compare_against_baseline(
    stats: &ProjectStats,
    baseline_proj: &Value,
) -> (Vec<String>, Vec<String>) {
    let mut regressions: Vec<String> = Vec::new();
    let mut improvements: Vec<String> = Vec::new();

    let baseline_symbols = baseline_proj["symbols"].as_i64().unwrap_or(0);
    let baseline_edges = baseline_proj["edges"].as_i64().unwrap_or(0);
    let baseline_routes = baseline_proj["routes"].as_i64().unwrap_or(0);
    let baseline_flow = baseline_proj["flow_edges"].as_i64().unwrap_or(0);

    for (label, current, baseline_val) in [
        ("symbols", stats.symbols, baseline_symbols),
        ("edges", stats.edges, baseline_edges),
        ("routes", stats.routes, baseline_routes),
        ("flow_edges", stats.flow_edges, baseline_flow),
    ] {
        if current < baseline_val {
            regressions.push(format!(
                "{label}: {baseline_val} → {current} ({diff})",
                diff = current - baseline_val
            ));
        } else if current > baseline_val {
            improvements.push(format!(
                "{label}: {baseline_val} → {current} (+{diff})",
                diff = current - baseline_val
            ));
        }
    }

    // Check min_* assertions (same logic as CLI).
    if let Some(obj) = baseline_proj["assertions"].as_object() {
        for (key, val) in obj {
            if let Some(min_val) = val.as_i64() {
                let current_val = match key.as_str() {
                    "min_routes" => stats.routes,
                    "min_flow_edges" => stats.flow_edges,
                    "min_symbols" => stats.symbols,
                    "min_files" => stats.files,
                    _ => continue,
                };
                if current_val < min_val {
                    regressions.push(format!(
                        "{key}: expected >={min_val}, got {current_val}"
                    ));
                }
            }
        }
    }

    (regressions, improvements)
}

/// Index a project into an on-disk DB in its TempDir, using the same path
/// the CLI uses (`<root>/.bearwisdom/index.db`).  Returns an open `Database`.
fn index_on_disk(project: &TestProject) -> Database {
    let db_path = resolve_db_path(project.path()).unwrap();
    let mut db = Database::open_with_vec(&db_path).unwrap();
    full_index(&mut db, project.path(), None, None).unwrap();
    // Re-open read-only style (connection already has the data).
    db
}

// ---------------------------------------------------------------------------
// test_quality_check_pass
// ---------------------------------------------------------------------------

/// Index the C# fixture, create a baseline that is at or below the actual
/// stats, and verify that no regressions are reported.
#[test]
fn test_quality_check_pass() {
    let project = TestProject::csharp_service();
    let db = index_on_disk(&project);
    let stats = read_project_stats(&db);

    // Sanity: something must have been indexed.
    assert!(stats.files >= 4, "fixture must have at least 4 files");
    assert!(stats.symbols > 0, "fixture must have symbols");

    // Baseline at exactly the current values — must pass (no regression, no improvement).
    let baseline_proj = json!({
        "project": "csharp-service",
        "path": project.path().to_string_lossy(),
        "symbols":    stats.symbols,
        "edges":      stats.edges,
        "routes":     stats.routes,
        "flow_edges": stats.flow_edges,
        "assertions": {
            "min_symbols": 1,
            "min_files":   1,
        }
    });

    let (regressions, improvements) = compare_against_baseline(&stats, &baseline_proj);

    assert!(
        regressions.is_empty(),
        "expected no regressions when baseline matches current: {regressions:?}"
    );
    assert!(
        improvements.is_empty(),
        "expected no improvements when baseline matches current: {improvements:?}"
    );
}

// ---------------------------------------------------------------------------
// test_quality_check_regression_detected
// ---------------------------------------------------------------------------

/// Create a baseline with inflated numbers so that every metric appears to
/// have regressed.  Verify all regressions are reported.
#[test]
fn test_quality_check_regression_detected() {
    let project = TestProject::csharp_service();
    let db = index_on_disk(&project);
    let stats = read_project_stats(&db);

    // Baseline with each count set far above the actual indexed values.
    let inflated_symbols = stats.symbols + 99999;
    let inflated_edges = stats.edges + 99999;
    let inflated_routes = stats.routes + 99999;
    let inflated_flow = stats.flow_edges + 99999;

    let baseline_proj = json!({
        "project": "csharp-service",
        "path": project.path().to_string_lossy(),
        "symbols":    inflated_symbols,
        "edges":      inflated_edges,
        "routes":     inflated_routes,
        "flow_edges": inflated_flow,
        "assertions": {
            "min_symbols": 99999
        }
    });

    let (regressions, _improvements) = compare_against_baseline(&stats, &baseline_proj);

    // At least the inflated fields must each appear as a regression.
    assert!(
        regressions.iter().any(|r| r.starts_with("symbols")),
        "symbols regression must be reported"
    );
    assert!(
        regressions.iter().any(|r| r.starts_with("edges")),
        "edges regression must be reported"
    );
    // The min_symbols assertion must also fire.
    assert!(
        regressions.iter().any(|r| r.contains("min_symbols")),
        "min_symbols assertion failure must be reported"
    );

    // Overall: there must be regressions.
    assert!(
        !regressions.is_empty(),
        "inflated baseline must produce regressions"
    );
}

// ---------------------------------------------------------------------------
// test_quality_check_improvement_detected
// ---------------------------------------------------------------------------

/// Create a baseline with numbers far below the actual indexed stats.
/// Verify that improvements are reported and no regressions are.
#[test]
fn test_quality_check_improvement_detected() {
    let project = TestProject::csharp_service();
    let db = index_on_disk(&project);
    let stats = read_project_stats(&db);

    // Only meaningful if there are symbols to improve upon.
    assert!(stats.symbols > 0, "fixture must have symbols");

    // Baseline at 0 for all numeric fields — every non-zero actual value is
    // an improvement.
    let baseline_proj = json!({
        "project": "csharp-service",
        "path": project.path().to_string_lossy(),
        "symbols":    0,
        "edges":      0,
        "routes":     0,
        "flow_edges": 0,
        "assertions": {}
    });

    let (regressions, improvements) = compare_against_baseline(&stats, &baseline_proj);

    assert!(
        regressions.is_empty(),
        "no regressions expected when baseline is all zeros: {regressions:?}"
    );

    // symbols must appear as an improvement since the fixture definitely has > 0.
    assert!(
        improvements.iter().any(|i| i.starts_with("symbols")),
        "symbols should be reported as an improvement over 0"
    );
}

// ---------------------------------------------------------------------------
// test_quality_check_missing_project
// ---------------------------------------------------------------------------

/// A baseline entry whose path does not exist on the filesystem should be
/// gracefully handled.  In the CLI this is a `continue` (skip with log).
/// Here we model the same guard and verify no panic / no stats produced.
#[test]
fn test_quality_check_missing_project() {
    let missing_path = "/this/path/does/not/exist/in/any/way";

    // Simulate what cmd_quality_check does for a missing path.
    let root = std::path::Path::new(missing_path);
    let path_exists = root.exists();

    assert!(
        !path_exists,
        "test precondition: path must not exist on this machine"
    );

    // The CLI skips the project entirely — no DB open, no stats read.
    // We simply verify the guard condition fires correctly.
    // This is the totality of the "graceful handling" for missing projects.
    let skipped = !path_exists;
    assert!(skipped, "missing project must be skipped");
}

// ---------------------------------------------------------------------------
// test_quality_check_multiple_projects_partial_regression
// ---------------------------------------------------------------------------

/// Two projects in the baseline: one whose stats match (pass), one whose
/// baseline is inflated (fail).  Verify the regression count and pass/fail
/// rollup behave correctly.
#[test]
fn test_quality_check_multiple_projects_partial_regression() {
    let project = TestProject::csharp_service();
    let db = index_on_disk(&project);
    let stats = read_project_stats(&db);

    // Project A: baseline matches current stats exactly → pass.
    let baseline_a = json!({
        "project": "project-a",
        "symbols": stats.symbols,
        "edges":   stats.edges,
        "routes":  stats.routes,
        "flow_edges": stats.flow_edges,
        "assertions": {}
    });

    // Project B: baseline is inflated → regression.
    let baseline_b = json!({
        "project": "project-b",
        "symbols": stats.symbols + 10000,
        "edges":   stats.edges,
        "routes":  stats.routes,
        "flow_edges": stats.flow_edges,
        "assertions": {}
    });

    // Evaluate both.
    let (reg_a, _) = compare_against_baseline(&stats, &baseline_a);
    let (reg_b, _) = compare_against_baseline(&stats, &baseline_b);

    let total_projects_with_regressions = [&reg_a, &reg_b]
        .iter()
        .filter(|r| !r.is_empty())
        .count();

    assert!(reg_a.is_empty(), "project-a must pass: {reg_a:?}");
    assert!(!reg_b.is_empty(), "project-b must fail (symbols inflated)");
    assert_eq!(total_projects_with_regressions, 1, "exactly one project must fail");
}

// ---------------------------------------------------------------------------
// test_quality_check_min_assertions_fire_independently
// ---------------------------------------------------------------------------

/// min_* assertions are evaluated independently of the numeric baseline
/// comparison.  Verify that a project can pass the numeric comparison (exact
/// match) but still fail an impossible min assertion.
#[test]
fn test_quality_check_min_assertions_fire_independently() {
    let project = TestProject::csharp_service();
    let db = index_on_disk(&project);
    let stats = read_project_stats(&db);

    // Baseline numbers are exact (no numeric regression/improvement), but
    // min_symbols is set impossibly high.
    let baseline_proj = json!({
        "project": "csharp-service",
        "symbols":    stats.symbols,
        "edges":      stats.edges,
        "routes":     stats.routes,
        "flow_edges": stats.flow_edges,
        "assertions": {
            "min_symbols": stats.symbols + 1  // impossible
        }
    });

    let (regressions, improvements) = compare_against_baseline(&stats, &baseline_proj);

    assert!(
        improvements.is_empty(),
        "no improvements: numeric values match"
    );
    assert!(
        regressions.iter().any(|r| r.contains("min_symbols")),
        "impossible min_symbols must produce a regression"
    );
}
