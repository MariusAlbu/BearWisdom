use super::*;
use bearwisdom::query::full_trace::{FullTraceResult, TraceNode, TraceRoot};
use bearwisdom::search::flow::FlowStep;
use bearwisdom::search::grep::GrepMatch;
use bearwisdom::{IndexFreshness, RefreshState, SearchResult};

fn make_search_result(file: &str, line: u32) -> SearchResult {
    SearchResult {
        name: "DoThing".to_string(),
        qualified_name: "App.DoThing".to_string(),
        kind: "function".to_string(),
        file_path: file.to_string(),
        start_line: line,
        signature: None,
        score: 1.0,
    }
}

fn make_grep_match(file: &str, line_number: u32, content: &str) -> GrepMatch {
    GrepMatch {
        file_path: file.to_string(),
        line_number,
        column: 0,
        line_content: content.to_string(),
        match_start: 0,
        match_end: 0,
    }
}

#[test]
fn search_single_result_inlines_path_and_omits_files_section() {
    let results = vec![make_search_result("src/foo.rs", 42)];
    let out = search(&results, 50);

    assert!(out.contains("#format:compact-v1"));
    assert!(
        !out.contains("#files"),
        "single-result must not emit a #files registry, got:\n{out}"
    );
    assert!(
        !out.contains("F1:"),
        "single-result must not produce F1 references, got:\n{out}"
    );
    assert!(
        out.contains("src/foo.rs:42"),
        "single-result must inline the path, got:\n{out}"
    );
}

#[test]
fn search_multi_result_uses_files_registry() {
    let results = vec![
        make_search_result("src/a.rs", 1),
        make_search_result("src/b.rs", 2),
    ];
    let out = search(&results, 50);

    assert!(
        out.contains("#files"),
        "multi-result must keep the file registry, got:\n{out}"
    );
    assert!(out.contains("F1:src/a.rs"));
    assert!(out.contains("F2:src/b.rs"));
}

#[test]
fn with_freshness_header_injects_index_block_into_compact_response() {
    let response = format!("#format:compact-v1\n#meta\ncount:0\n");
    let out = crate::server::BearWisdomServer::with_freshness_header(
        response,
        Some(IndexFreshness {
            last_complete_at_ms: Some(1_700_000_000_000),
            refresh_state: RefreshState::Disabled,
            refresh_enabled: false,
            watching: false,
        }),
    );
    assert!(out.starts_with("#format:compact-v1\n"));
    assert!(out.contains("#index"));
    assert!(out.contains("last_complete_at_ms:1700000000000"));
    assert!(out.contains("age_ms:"));
    assert!(out.contains("refresh_state:disabled"));
    assert!(out.contains("mode:query_only"));
    assert!(out.contains("#meta"));
}

#[test]
fn with_freshness_header_skips_non_compact_responses() {
    let response = r#"{"ok":true,"data":[]}"#.to_string();
    let out = crate::server::BearWisdomServer::with_freshness_header(response, None);
    assert_eq!(out, r#"{"ok":true,"data":[]}"#);
}

#[test]
fn with_freshness_header_handles_unknown_index_time() {
    let response = format!("#format:compact-v1\n#meta\ncount:0\n");
    let out = crate::server::BearWisdomServer::with_freshness_header(response, None);
    assert!(out.contains("last_complete_at_ms:unknown"));
}

#[test]
fn grep_single_result_inlines_path() {
    let results = vec![make_grep_match(
        "crates/foo/src/lib.rs",
        10,
        "fn hello() {}",
    )];
    let out = grep(&results, 50);

    assert!(
        !out.contains("#files"),
        "single-result grep must skip #files, got:\n{out}"
    );
    assert!(out.contains("crates/foo/src/lib.rs:10"));
}

#[test]
fn search_capped_at_limit_sets_truncated_flag() {
    // Caller asked for 3, got exactly 3 — could be more upstream → truncated.
    let results = vec![
        make_search_result("a.rs", 1),
        make_search_result("b.rs", 2),
        make_search_result("c.rs", 3),
    ];
    let out = search(&results, 3);
    assert!(
        out.contains("truncated:true"),
        "expected truncation flag, got:\n{out}"
    );
}

#[test]
fn search_under_limit_omits_truncated_flag() {
    let results = vec![make_search_result("a.rs", 1), make_search_result("b.rs", 2)];
    let out = search(&results, 50);
    assert!(
        !out.contains("truncated:true"),
        "should not flag truncation when under limit, got:\n{out}"
    );
}

#[test]
fn flow_trace_preserves_direction_edge_provenance_and_file_registry() {
    let forward = vec![
        FlowStep {
            edge_id: 10,
            parent_edge_id: None,
            depth: 0,
            file_path: "web/client.ts".to_string(),
            line: Some(12),
            symbol: Some("loadUser".to_string()),
            language: "typescript".to_string(),
            target_file_path: Some("api/user.rs".to_string()),
            target_line: Some(44),
            target_symbol: Some("get_user".to_string()),
            target_language: Some("rust".to_string()),
            paired: true,
            edge_type: "http_call".to_string(),
            protocol: Some("http".to_string()),
            http_method: Some("GET".to_string()),
            url_pattern: Some("/users/{id}".to_string()),
            confidence: 0.95,
        },
        FlowStep {
            edge_id: 11,
            parent_edge_id: Some(10),
            depth: 1,
            file_path: "api/user.rs".to_string(),
            line: Some(44),
            symbol: Some("get_user".to_string()),
            language: "rust".to_string(),
            target_file_path: None,
            target_line: None,
            target_symbol: None,
            target_language: None,
            paired: false,
            edge_type: "route_handler".to_string(),
            protocol: Some("http".to_string()),
            http_method: None,
            url_pattern: Some("/users/{id}".to_string()),
            confidence: 0.7,
        },
    ];
    let out = flow_trace("both", &forward, &[], 80);

    assert!(out.contains("evidence:flow_edges"));
    assert!(out.contains("paired:1|single_ended:1"));
    assert!(out.contains("incomplete:true"));
    assert!(out.contains("#forward"));
    assert!(out.contains("F1:web/client.ts"));
    assert!(out.contains("F2:api/user.rs"));
    assert!(out.contains("E11|E10|d1"));
    assert!(out.contains("http_call|http"));
}

#[test]
fn empty_flow_trace_is_explicitly_inconclusive() {
    let out = flow_trace("both", &[], &[], 80);
    assert!(out.contains("empty_is_inconclusive:true"));
}

#[test]
fn full_trace_encodes_parent_links_and_flow_jumps() {
    let child = TraceNode {
        name: "save".to_string(),
        qualified_name: "repo::save".to_string(),
        kind: "function".to_string(),
        file_path: "src/repo.rs".to_string(),
        line: 20,
        edge_kind: "http_call".to_string(),
        depth: 1,
        children: vec![],
    };
    let entry = TraceNode {
        name: "handle".to_string(),
        qualified_name: "api::handle".to_string(),
        kind: "function".to_string(),
        file_path: "src/api.rs".to_string(),
        line: 10,
        edge_kind: "entry_point".to_string(),
        depth: 0,
        children: vec![child],
    };
    let result = FullTraceResult {
        traces: vec![TraceRoot {
            entry,
            node_count: 2,
        }],
        total_symbols: 2,
        flow_jumps: 1,
        incomplete_flow_edges: 1,
    };
    let out = full_trace(&result, 100);

    assert!(out.contains("flow_jumps:1"));
    assert!(out.contains("incomplete_flow_edges:1"));
    assert!(out.contains("incomplete:true"));
    assert!(out.contains("N1|-|d0|entry_point|api::handle"));
    assert!(out.contains("N2|N1|d1|http_call|repo::save"));
}

#[test]
fn investigate_keeps_resolved_and_unresolved_occurrences_distinct() {
    use bearwisdom::query::investigate::{InvestigateResult, SlimSymbol};
    use bearwisdom::query::references::{
        OccurrenceCoverage, ReferenceCoverage, ResolvedReferenceSource, SourceAttestedOccurrence,
    };

    let result = InvestigateResult {
        symbol: SlimSymbol {
            symbol_id: Some("rust:module::target".to_string()),
            name: "target".to_string(),
            qualified_name: "module::target".to_string(),
            kind: "function".to_string(),
            file_path: "lib.rs".to_string(),
            line: 1,
            signature: None,
        },
        references: vec![],
        source_attested_unresolved: vec![SourceAttestedOccurrence {
            referencing_symbol: "module::caller".to_string(),
            referencing_kind: "function".to_string(),
            file_path: "lib.rs".to_string(),
            line: 12,
            column: 7,
            target_name: "target".to_string(),
            edge_kind: "calls".to_string(),
            outcome: "unresolved".to_string(),
            candidate_declaration_count: 1,
            evidence_source: "ref_resolution_log".to_string(),
        }],
        reference_coverage: ReferenceCoverage {
            occurrence_coverage: OccurrenceCoverage::Complete,
            internal_files: 1,
            measured_files: 1,
            missing_files: 0,
            stale_files: 0,
            resolved_source: ResolvedReferenceSource::RefResolutionLog,
            resolved_total: 0,
            resolved_truncated: false,
            source_attested_total: 1,
            source_attested_truncated: false,
        },
        callers: vec![],
        callees: vec![],
        blast_radius: None,
    };
    let out = investigate(&result);

    assert!(out.contains("rust:module::target|module::target"));
    assert!(out.contains("references:0|resolved_total:0"));
    assert!(out.contains("source_attested_unresolved:1"));
    assert!(out.contains("reference_coverage:complete"));
    assert!(out.contains("#source_attested_unresolved"));
    assert!(out.contains("12:7|calls|unresolved|candidates:1"));
}
