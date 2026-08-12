// =============================================================================
// quality_recapture.rs — re-index the baseline corpus and write fresh metrics.
//
// Walks the project list in the baseline file, re-indexes every entry that has
// source on disk, and replaces its metrics in place. Entries whose source is
// gone keep their previous values; when the run was scoped with `--project`,
// those same entries become failures instead, because the caller named them.
// =============================================================================

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bearwisdom::db::Database;

use crate::recapture_entry::{self, IndexPerf};
use crate::recapture_requests::{failure_summary, failures_json, RecaptureRequests, SkipReason};

/// What prevents a project root from being indexed, or `None` when it can be.
///
/// A root that exists but holds no non-hidden entry is a ghost: the source was
/// deleted and only cache directories such as `.bearwisdom/` remain. Indexing
/// it would replace every metric with zero, so capture stops here.
pub(crate) fn capture_blocker(root: &Path) -> Option<SkipReason> {
    if !root.exists() {
        return Some(SkipReason::PathMissing);
    }
    if crate::is_ghost_project(root) {
        return Some(SkipReason::GhostSource);
    }
    None
}

/// Recapture the quality baseline.
///
/// Preserves each entry's `project`, `path`, and identity fields and refreshes
/// its metric and assertion blocks from a fresh index. Projects outside a
/// `--project` scope pass through untouched — the file is rewritten in full
/// with only the targeted entries replaced.
///
/// Returns an error when the run was scoped and any requested project did not
/// reach fresh metrics; the baseline is still written first so the projects
/// that did capture are not lost to the failure.
pub(crate) fn run(baseline_path: &str, only_projects: &[String]) -> Result<String> {
    let baseline_file = PathBuf::from(baseline_path);
    let content = std::fs::read_to_string(&baseline_file)
        .with_context(|| format!("Failed to read baseline: {}", baseline_file.display()))?;
    let mut baseline: serde_json::Value =
        serde_json::from_str(&content).context("Failed to parse baseline JSON")?;

    let Some(projects) = baseline["projects"].as_array().cloned() else {
        anyhow::bail!("baseline.projects is not an array");
    };

    let mut requests = RecaptureRequests::new(only_projects);
    let mut new_projects: Vec<serde_json::Value> = Vec::with_capacity(projects.len());
    let mut recaptured = 0u32;
    let mut skipped_missing = 0u32;
    let mut skipped_ghost = 0u32;

    for proj in projects {
        let name = proj["project"].as_str().unwrap_or("?").to_string();
        if !requests.selects(&name) {
            new_projects.push(proj);
            continue;
        }
        let proj_path = proj["path"].as_str().unwrap_or("").to_string();
        let root = PathBuf::from(&proj_path);

        eprint!("--- {name} ---\n  ");

        if let Some(reason) = capture_blocker(&root) {
            eprintln!("SKIP ({})", reason.detail(&proj_path));
            match reason {
                SkipReason::PathMissing => skipped_missing += 1,
                SkipReason::GhostSource => skipped_ghost += 1,
            }
            requests.record_skip(&name, &proj_path, reason);
            new_projects.push(proj);
            continue;
        }

        eprintln!("Reindexing...");
        new_projects.push(capture_project(&name, &root, &proj)?);
        recaptured += 1;
    }

    baseline["projects"] = serde_json::Value::Array(new_projects);

    // A run that captured nothing leaves the file alone: rewriting it would
    // stamp `captured_at` with a capture that never happened.
    if recaptured > 0 {
        baseline["captured_at"] = serde_json::json!(format!("{}T00:00:00Z", utc_date_stamp()));
        let serialized =
            serde_json::to_string_pretty(&baseline).context("Failed to serialize baseline JSON")?;
        std::fs::write(&baseline_file, serialized)
            .with_context(|| format!("Failed to write baseline: {}", baseline_file.display()))?;
    }

    let failures = requests.into_failures();
    let file_state = if recaptured > 0 {
        format!("Wrote {}", baseline_file.display())
    } else {
        format!("Left {} unchanged", baseline_file.display())
    };
    eprintln!(
        "\n=== RECAPTURE: {recaptured} re-indexed, {skipped_missing} missing, {skipped_ghost} ghost, {} failed ===\n\
         {file_state} ({} total projects)",
        failures.len(),
        baseline["projects"].as_array().map(|a| a.len()).unwrap_or(0)
    );

    if !failures.is_empty() {
        anyhow::bail!(
            "{} requested project(s) produced no metrics:\n{}",
            failures.len(),
            failure_summary(&failures)
        );
    }

    crate::ok_json(serde_json::json!({
        "recaptured": recaptured,
        "skipped_missing": skipped_missing,
        "skipped_ghost": skipped_ghost,
        "failures": failures_json(&failures),
        "baseline": baseline_file.display().to_string(),
    }))
}

/// Re-index one project and return its refreshed baseline entry.
///
/// Opening the database migrates it to the current schema, so an entry that
/// captures here is also the point at which a stale index file is brought
/// forward. Indexing time is measured around `full_index` alone so the perf
/// block excludes DB open, stats queries, and JSON writes.
fn capture_project(
    name: &str,
    root: &Path,
    existing: &serde_json::Value,
) -> Result<serde_json::Value> {
    let db_path = bearwisdom::resolve_db_path(root)?;
    let mut db =
        Database::open(&db_path).with_context(|| format!("Failed to open DB for {name}"))?;

    let index_start = std::time::Instant::now();
    bearwisdom::full_index(&mut db, root, None, None, None)
        .with_context(|| format!("Index failed for {name}"))?;
    let perf = IndexPerf {
        duration_ms: index_start.elapsed().as_millis() as u64,
        end_ws_mb: recapture_entry::current_working_set_mb(),
    };

    let stats = bearwisdom::index_stats(&db)?;
    let flow_edge_types: std::collections::BTreeMap<String, u32> =
        bearwisdom::flow_edge_breakdown(&db)?
            .into_iter()
            .map(|b| (b.edge_type, b.count))
            .collect();
    let rb = bearwisdom::resolution_breakdown(&db)?;

    eprintln!(
        "  OK ({} files, {} symbols, {} int_edges, {:.1}% resolved, {} ms, {} MB end_ws)",
        stats.file_count,
        stats.symbol_count,
        rb.internal_edges,
        rb.resolution_rate,
        perf.duration_ms,
        perf.end_ws_mb
    );

    Ok(recapture_entry::snapshot(
        existing,
        &stats,
        &rb,
        &flow_edge_types,
        perf,
    ))
}

/// Today's UTC date as `YYYY-MM-DD`.
///
/// Derived from the UNIX epoch via Howard Hinnant's civil-from-days algorithm.
/// Time of day is dropped — baselines are recaptured by hand, not on a clock.
fn utc_date_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    civil_from_days((secs / 86_400) as i64)
}

/// Convert days since 1970-01-01 into a `YYYY-MM-DD` stamp.
fn civil_from_days(days: i64) -> String {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

#[cfg(test)]
#[path = "quality_recapture_tests.rs"]
mod tests;
