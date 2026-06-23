use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::indexer::resolve::engine::trace;
use crate::type_checker::core::types::TypeArena;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, MemberChain, ChainSegment,
    ParsedFile, SegmentKind, SymbolKind, Visibility,
};

// Global mutex so trace tests, which share process-global atomic state, never
// run concurrently with each other inside the same test binary.
static TRACE_TEST_LOCK: Mutex<()> = Mutex::new(());

fn esym(name: &str, qname: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.into(),
        qualified_name: qname.into(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn cseg(name: &str, is_call: bool) -> ChainSegment {
    ChainSegment {
        name: name.into(),
        node_kind: String::new(),
        kind: SegmentKind::Identifier,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }
}

fn chain_ref(src: usize, target: &str, segments: Vec<ChainSegment>) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: src,
        target_name: target.into(),
        kind: EdgeKind::Calls,
        line: 1,
        col: 0,
        module: None,
        chain: Some(MemberChain { segments }),
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

/// A chain ref `a.b()` where `a` has no type in any cache — the root is UNTYPABLE.
/// The trace must capture ROOT UNTYPABLE and RESULT UNRESOLVED.
#[test]
fn trace_captures_untypable_root_and_unresolved_result() {
    let _guard = TRACE_TEST_LOCK.lock().unwrap();
    // Two symbols: a function host and a method we want to call.
    let symbols = vec![
        esym("host", "host", SymbolKind::Function),        // 0
        esym("NoSuchClass", "NoSuchClass", SymbolKind::Class), // 1 — not in scope of `a`
    ];
    // One chain ref: `a.b()` where `a` has no type. Root is UNTYPABLE.
    let refs = vec![chain_ref(
        0,
        "b",
        vec![
            cseg("a", false),
            cseg("b", true),
        ],
    )];
    let pf = ParsedFile {
        path: "trace_test.ts".into(),
        language: "typescript".into(),
        content_hash: String::new(),
        size: 0,
        line_count: 2,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("trace_test.ts".to_string(), "host".to_string()), 1i64);
    id_map.insert(("trace_test.ts".to_string(), "NoSuchClass".to_string()), 2i64);

    let arena = Arc::new(TypeArena::new());
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        std::slice::from_ref(&pf),
        &id_map,
        Arc::clone(&arena),
    );
    let profiles = super::build_profiles();
    let solver = super::SemanticModel::production();

    // Activate trace with a filter matching this file and line.
    trace::set_filter("trace_test.ts".to_string(), 1, String::new());
    trace::activate();
    super::resolve_one_file(&pf, &tree, &profiles, &solver, &id_map);
    trace::deactivate();
    let collected = trace::drain_collected();
    trace::clear_filter();

    assert!(
        !collected.is_empty(),
        "expected at least one traced ref; none collected"
    );

    let all_lines: Vec<&str> = collected
        .iter()
        .flat_map(|t| t.trace_lines.iter().map(String::as_str))
        .collect();

    assert!(
        all_lines.iter().any(|l| l.contains("ROOT") && l.contains("UNTYPABLE")),
        "expected a ROOT UNTYPABLE line; got: {all_lines:?}"
    );
    assert!(
        all_lines.iter().any(|l| l.contains("RESULT") && l.contains("UNRESOLVED")),
        "expected a RESULT UNRESOLVED line; got: {all_lines:?}"
    );
}

/// A bare (non-chain) ref to a name that exists — the REF header and RESULT resolved
/// lines must both appear in the trace, and SEED none must appear since no flow binding.
#[test]
fn trace_captures_resolved_ref_and_seed_none() {
    let _guard = TRACE_TEST_LOCK.lock().unwrap();
    let symbols = vec![
        esym("caller", "caller", SymbolKind::Function),   // 0
        esym("Target", "Target", SymbolKind::Class),       // 1
    ];
    // A bare TypeRef from `caller` to `Target`.
    let refs = vec![ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: "Target".into(),
        kind: EdgeKind::TypeRef,
        line: 2,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 10,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }];
    let pf = ParsedFile {
        path: "trace_resolved.ts".into(),
        language: "typescript".into(),
        content_hash: String::new(),
        size: 0,
        line_count: 5,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("trace_resolved.ts".to_string(), "caller".to_string()), 10i64);
    id_map.insert(("trace_resolved.ts".to_string(), "Target".to_string()), 20i64);

    let arena = Arc::new(TypeArena::new());
    let tree = crate::indexer::resolve::engine::compilation::Compilation::build(
        std::slice::from_ref(&pf),
        &id_map,
        Arc::clone(&arena),
    );
    let profiles = super::build_profiles();
    let solver = super::SemanticModel::production();

    // Filter on line 2 (the Target ref).
    trace::set_filter("trace_resolved.ts".to_string(), 2, "Target".to_string());
    trace::activate();
    super::resolve_one_file(&pf, &tree, &profiles, &solver, &id_map);
    trace::deactivate();
    let collected = trace::drain_collected();
    trace::clear_filter();

    assert!(
        !collected.is_empty(),
        "expected at least one traced ref"
    );

    let all_lines: Vec<&str> = collected
        .iter()
        .flat_map(|t| t.trace_lines.iter().map(String::as_str))
        .collect();

    assert!(
        all_lines.iter().any(|l| l.contains("REF") && l.contains("Target")),
        "expected a REF header line for Target; got: {all_lines:?}"
    );
    assert!(
        all_lines.iter().any(|l| l.contains("RESULT") && l.contains("resolved")),
        "expected a RESULT resolved line; got: {all_lines:?}"
    );
    assert!(
        all_lines.iter().any(|l| l.contains("SEED") && l.contains("none")),
        "expected a SEED none line; got: {all_lines:?}"
    );
}

/// When TRACE_ACTIVE is false, take_ref() returns empty and drain_collected() returns empty —
/// the zero-cost gate leaves no residue.
#[test]
fn trace_inactive_produces_no_lines() {
    let _guard = TRACE_TEST_LOCK.lock().unwrap();
    // Ensure trace is off and any previous collected state is cleared.
    trace::deactivate();
    trace::clear_filter();
    let _ = trace::drain_collected();
    // Verify nothing leaks when the gate is off.
    assert!(
        !trace::TRACE_ACTIVE.load(std::sync::atomic::Ordering::Relaxed),
        "TRACE_ACTIVE must be false after deactivate"
    );
    // Calling take_ref() when no collector is installed must return empty.
    let lines = trace::take_ref();
    assert!(lines.is_empty(), "take_ref without begin_ref must be empty");
    // drain_collected must be empty when nothing was pushed.
    let collected = trace::drain_collected();
    assert!(collected.is_empty(), "drain_collected must be empty when trace was never active");
}
