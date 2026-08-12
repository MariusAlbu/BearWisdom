// =============================================================================
// recapture_entry.rs — build one baseline entry from a freshly indexed project.
//
// Takes the existing entry plus the stats read off the rebuilt database and
// returns the entry to write back: the consolidated metric block, the perf
// block, and the assertion floors rebased onto the captured values.
// =============================================================================

use std::collections::BTreeMap;

use bearwisdom::{IndexStats, ResolutionBreakdown};

/// Wall-clock and memory cost of one project's indexing pass.
///
/// `duration_ms` covers the indexer pipeline only (no DB open, stats queries,
/// or JSON writes), so a perf regression is attributable to the pipeline.
pub(crate) struct IndexPerf {
    pub(crate) duration_ms: u64,
    pub(crate) end_ws_mb: u64,
}

/// Current process working set in MiB (Windows: PSAPI; others: 0).
///
/// Sampled after a project's indexing pass — captures memory the indexer is
/// still holding once `full_index` returns, which is the retained floor rather
/// than the in-flight peak. Process-cumulative `PeakWorkingSetSize` is NOT used
/// because it only grows across a batch run, making every project after the
/// largest report the same number.
#[cfg(windows)]
pub(crate) fn current_working_set_mb() -> u64 {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let mut counters: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let ok = GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX) as *mut PROCESS_MEMORY_COUNTERS,
            size,
        );
        if ok == 0 {
            0
        } else {
            counters.WorkingSetSize as u64 / (1024 * 1024)
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn current_working_set_mb() -> u64 {
    0
}

/// Produce the baseline entry for a captured project.
///
/// `existing` supplies the fields the capture does not measure (`project`,
/// `path`, `corpus_class`, notes). Metric keys are overwritten, superseded
/// keys are dropped, and the assertion floors are rebased so the next
/// `quality-check` compares against the values just captured.
///
/// The metric block measures five quality dimensions: language detection
/// (`languages`), extraction + resolution (`internal_edges` /
/// `internal_unresolved` / `resolution_rate` plus the per-(language, kind)
/// breakdown that pinpoints a leaking extractor), connector wiring
/// (`flow_edges` / `flow_edge_types` / `routes`), dead-code trust
/// (transitively via `resolution_rate`), and doc drift (`code_chunks`).
pub(crate) fn snapshot(
    existing: &serde_json::Value,
    stats: &IndexStats,
    rb: &ResolutionBreakdown,
    flow_edge_types: &BTreeMap<String, u32>,
    perf: IndexPerf,
) -> serde_json::Value {
    let mut entry = existing.clone();

    // Keys superseded by the consolidated schema. Removal is unconditional so
    // an entry captured under the older shape converges on the current one.
    let superseded = [
        "edges",
        "unresolved_refs",
        "unresolved_ref_count",
        "external_ref_count",
    ];
    if let Some(obj) = entry.as_object_mut() {
        for k in superseded {
            obj.remove(k);
        }
    }

    entry["files"] = serde_json::json!(stats.file_count);
    entry["languages"] = serde_json::json!(rb.languages);
    entry["symbols"] = serde_json::json!(stats.symbol_count);
    entry["internal_edges"] = serde_json::json!(rb.internal_edges);
    entry["internal_unresolved"] = serde_json::json!(rb.internal_unresolved);
    entry["resolution_rate"] = serde_json::json!(rb.resolution_rate);
    // Counters that are zero for most projects are written only when non-zero,
    // keeping the entry readable; readers treat an absent key as zero.
    if rb.generated_excluded > 0 {
        entry["generated_excluded"] = serde_json::json!(rb.generated_excluded);
    }
    if rb.drained_refs > 0 {
        entry["drained_refs"] = serde_json::json!(rb.drained_refs);
    }
    if rb.vendored_files_reclassified > 0 {
        entry["vendored_files_reclassified"] = serde_json::json!(rb.vendored_files_reclassified);
    }
    if rb.generated_files_reclassified > 0 {
        entry["generated_files_reclassified"] = serde_json::json!(rb.generated_files_reclassified);
    }
    entry["unresolved_by_lang_kind"] = serde_json::json!(rb.unresolved_by_lang_kind);
    entry["rate_by_language"] = serde_json::json!(rb.rate_by_language);
    entry["flow_edges"] = serde_json::json!(stats.flow_edge_count);
    entry["flow_edge_types"] = serde_json::json!(flow_edge_types);
    entry["routes"] = serde_json::json!(stats.route_count);
    entry["code_chunks"] = serde_json::json!(rb.code_chunks);
    // Kept apart from the correctness metrics so a perf regression cannot
    // masquerade as a resolution regression.
    entry["perf"] = serde_json::json!({
        "index_duration_ms": perf.duration_ms,
        "end_ws_mb": perf.end_ws_mb,
        "files_per_sec": if perf.duration_ms > 0 {
            (stats.file_count as f64 * 1000.0 / perf.duration_ms as f64).round() as u64
        } else { 0 },
    });

    refresh_assertions(&mut entry, stats, rb, flow_edge_types);
    entry
}

/// Rebase the entry's assertion floors onto the captured values.
///
/// Only keys already present are updated, so a project asserts exactly the
/// dimensions it opted into. `min_resolution_rate` is the exception: it is
/// added when absent, at the integer floor of the captured rate so decimal
/// jitter between runs does not read as a regression.
fn refresh_assertions(
    entry: &mut serde_json::Value,
    stats: &IndexStats,
    rb: &ResolutionBreakdown,
    flow_edge_types: &BTreeMap<String, u32>,
) {
    let Some(assertions) = entry.as_object_mut().and_then(|o| {
        o.entry("assertions".to_string())
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
    }) else {
        return;
    };

    let existing_keys: Vec<String> = assertions.keys().cloned().collect();
    for key in existing_keys {
        let Some(new_value) = assertion_value(&key, stats, rb, flow_edge_types) else {
            continue;
        };
        assertions.insert(key, new_value);
    }
    if !assertions.contains_key("min_resolution_rate") {
        assertions.insert(
            "min_resolution_rate".to_string(),
            serde_json::json!(rb.resolution_rate.floor() as u32),
        );
    }
}

/// The captured value backing one assertion key, or `None` for a key this
/// capture does not measure (which leaves the existing floor untouched).
fn assertion_value(
    key: &str,
    stats: &IndexStats,
    rb: &ResolutionBreakdown,
    flow_edge_types: &BTreeMap<String, u32>,
) -> Option<serde_json::Value> {
    match key {
        "min_routes" => Some(serde_json::json!(stats.route_count)),
        "min_flow_edges" => Some(serde_json::json!(stats.flow_edge_count)),
        "min_edges" => Some(serde_json::json!(rb.internal_edges)),
        "min_symbols" => Some(serde_json::json!(stats.symbol_count)),
        "min_files" => Some(serde_json::json!(stats.file_count)),
        "min_resolution_rate" => Some(serde_json::json!(rb.resolution_rate.floor() as u32)),
        // Flow-edge-type floors, `min_{type}_edges`. A type missing from the
        // capture means the connector produced zero, recorded as 0 rather than
        // keeping the old floor — otherwise every later run flags the same
        // regression against a threshold nothing can reach.
        _ => {
            let ty = key.strip_prefix("min_")?.strip_suffix("_edges")?;
            Some(serde_json::json!(flow_edge_types.get(ty).copied().unwrap_or(0)))
        }
    }
}

#[cfg(test)]
#[path = "recapture_entry_tests.rs"]
mod tests;
