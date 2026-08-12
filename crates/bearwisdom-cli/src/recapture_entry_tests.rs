// Sibling test file for `recapture_entry.rs`. Covers the shape of a captured
// baseline entry: which keys are written, which are dropped, and how the
// assertion floors are rebased onto the values just measured.

use super::*;

fn stats() -> IndexStats {
    IndexStats {
        file_count: 120,
        symbol_count: 4_500,
        route_count: 12,
        flow_edge_count: 88,
        ..Default::default()
    }
}

fn breakdown() -> ResolutionBreakdown {
    ResolutionBreakdown {
        internal_edges: 9_000,
        internal_unresolved: 1_000,
        external_known_unhydrated: 0,
        generated_excluded: 0,
        drained_refs: 0,
        vendored_files_reclassified: 0,
        generated_files_reclassified: 0,
        internal_resolution_rate: 90.0,
        precision: 90.0,
        resolution_rate: 90.5,
        unresolved_by_lang_kind: BTreeMap::from([("rust.calls".to_string(), 1_000u32)]),
        internal_edges_by_lang: BTreeMap::new(),
        rate_by_language: BTreeMap::from([("rust".to_string(), 90.5f64)]),
        unresolved_by_origin_language: BTreeMap::new(),
        unresolved_by_package: BTreeMap::new(),
        resolved_by_strategy: BTreeMap::new(),
        top_unresolved_targets: Vec::new(),
        low_confidence_edges: 0,
        low_confidence_threshold: 0.8,
        languages: BTreeMap::from([("rust".to_string(), 120u32)]),
        code_chunks: 42,
        drain_audit: Vec::new(),
    }
}

fn flow_types() -> BTreeMap<String, u32> {
    BTreeMap::from([("http".to_string(), 60u32), ("queue".to_string(), 28u32)])
}

fn perf() -> IndexPerf {
    IndexPerf {
        duration_ms: 2_000,
        end_ws_mb: 512,
    }
}

#[test]
fn snapshot_writes_consolidated_metric_block() {
    let existing = serde_json::json!({ "project": "demo", "path": "F:/demo" });
    let entry = snapshot(&existing, &stats(), &breakdown(), &flow_types(), perf());

    assert_eq!(entry["files"], 120);
    assert_eq!(entry["symbols"], 4_500);
    assert_eq!(entry["internal_edges"], 9_000);
    assert_eq!(entry["internal_unresolved"], 1_000);
    assert_eq!(entry["resolution_rate"], 90.5);
    assert_eq!(entry["routes"], 12);
    assert_eq!(entry["flow_edges"], 88);
    assert_eq!(entry["code_chunks"], 42);
    assert_eq!(entry["languages"]["rust"], 120);
    assert_eq!(entry["rate_by_language"]["rust"], 90.5);
    assert_eq!(entry["unresolved_by_lang_kind"]["rust.calls"], 1_000);
    assert_eq!(entry["flow_edge_types"]["http"], 60);
}

#[test]
fn snapshot_preserves_fields_the_capture_does_not_measure() {
    let existing = serde_json::json!({
        "project": "demo",
        "path": "F:/demo",
        "corpus_class": "framework-source",
    });
    let entry = snapshot(&existing, &stats(), &breakdown(), &flow_types(), perf());

    assert_eq!(entry["project"], "demo");
    assert_eq!(entry["path"], "F:/demo");
    assert_eq!(entry["corpus_class"], "framework-source");
}

#[test]
fn snapshot_drops_superseded_keys() {
    let existing = serde_json::json!({
        "project": "demo",
        "edges": 1,
        "unresolved_refs": 2,
        "unresolved_ref_count": 3,
        "external_ref_count": 4,
    });
    let entry = snapshot(&existing, &stats(), &breakdown(), &flow_types(), perf());

    for key in ["edges", "unresolved_refs", "unresolved_ref_count", "external_ref_count"] {
        assert!(entry.get(key).is_none(), "{key} should be dropped");
    }
}

#[test]
fn zero_valued_optional_counters_are_omitted() {
    let existing = serde_json::json!({ "project": "demo" });
    let entry = snapshot(&existing, &stats(), &breakdown(), &flow_types(), perf());

    for key in [
        "generated_excluded",
        "drained_refs",
        "vendored_files_reclassified",
        "generated_files_reclassified",
    ] {
        assert!(entry.get(key).is_none(), "{key} is zero, so it is omitted");
    }
}

#[test]
fn nonzero_optional_counters_are_written() {
    let mut rb = breakdown();
    rb.generated_excluded = 7;
    rb.drained_refs = 11;
    rb.vendored_files_reclassified = 3;
    rb.generated_files_reclassified = 5;

    let entry = snapshot(
        &serde_json::json!({ "project": "demo" }),
        &stats(),
        &rb,
        &flow_types(),
        perf(),
    );

    assert_eq!(entry["generated_excluded"], 7);
    assert_eq!(entry["drained_refs"], 11);
    assert_eq!(entry["vendored_files_reclassified"], 3);
    assert_eq!(entry["generated_files_reclassified"], 5);
}

#[test]
fn perf_block_reports_duration_memory_and_throughput() {
    let entry = snapshot(
        &serde_json::json!({ "project": "demo" }),
        &stats(),
        &breakdown(),
        &flow_types(),
        perf(),
    );

    assert_eq!(entry["perf"]["index_duration_ms"], 2_000);
    assert_eq!(entry["perf"]["end_ws_mb"], 512);
    // 120 files over 2s.
    assert_eq!(entry["perf"]["files_per_sec"], 60);
}

#[test]
fn zero_duration_yields_zero_throughput_rather_than_dividing_by_zero() {
    let entry = snapshot(
        &serde_json::json!({ "project": "demo" }),
        &stats(),
        &breakdown(),
        &flow_types(),
        IndexPerf {
            duration_ms: 0,
            end_ws_mb: 1,
        },
    );

    assert_eq!(entry["perf"]["files_per_sec"], 0);
}

#[test]
fn existing_assertion_floors_rebase_onto_captured_values() {
    let existing = serde_json::json!({
        "project": "demo",
        "assertions": {
            "min_files": 1,
            "min_symbols": 1,
            "min_edges": 1,
            "min_routes": 1,
            "min_flow_edges": 1,
            "min_resolution_rate": 1,
            "min_http_edges": 1,
        },
    });
    let entry = snapshot(&existing, &stats(), &breakdown(), &flow_types(), perf());
    let a = &entry["assertions"];

    assert_eq!(a["min_files"], 120);
    assert_eq!(a["min_symbols"], 4_500);
    assert_eq!(a["min_edges"], 9_000);
    assert_eq!(a["min_routes"], 12);
    assert_eq!(a["min_flow_edges"], 88);
    // Integer floor of 90.5 — decimal jitter must not read as a regression.
    assert_eq!(a["min_resolution_rate"], 90);
    assert_eq!(a["min_http_edges"], 60);
}

#[test]
fn absent_min_resolution_rate_is_added_at_the_captured_floor() {
    let entry = snapshot(
        &serde_json::json!({ "project": "demo", "assertions": {} }),
        &stats(),
        &breakdown(),
        &flow_types(),
        perf(),
    );

    assert_eq!(entry["assertions"]["min_resolution_rate"], 90);
}

#[test]
fn entry_without_assertions_gains_the_resolution_floor() {
    let entry = snapshot(
        &serde_json::json!({ "project": "demo" }),
        &stats(),
        &breakdown(),
        &flow_types(),
        perf(),
    );

    assert_eq!(entry["assertions"]["min_resolution_rate"], 90);
}

#[test]
fn flow_edge_type_floor_drops_to_zero_when_the_connector_produced_none() {
    // `min_grpc_edges` has no matching captured type: the connector emitted
    // nothing, so the floor records 0 instead of keeping an unreachable value.
    let existing = serde_json::json!({
        "project": "demo",
        "assertions": { "min_grpc_edges": 25 },
    });
    let entry = snapshot(&existing, &stats(), &breakdown(), &flow_types(), perf());

    assert_eq!(entry["assertions"]["min_grpc_edges"], 0);
}

#[test]
fn assertion_keys_the_capture_does_not_measure_are_left_alone() {
    let existing = serde_json::json!({
        "project": "demo",
        "assertions": { "max_index_seconds": 30 },
    });
    let entry = snapshot(&existing, &stats(), &breakdown(), &flow_types(), perf());

    assert_eq!(entry["assertions"]["max_index_seconds"], 30);
}
