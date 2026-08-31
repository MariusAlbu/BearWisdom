// =============================================================================
// javascript/flow_tests.rs — JS_FLOW_CONFIG against the JavaScript grammar
//
// Exercises the structural flow subset on plain-JS sources: initializer
// binding, await flagging, object-destructure seeding, instanceof/typeof
// narrowing, discriminant narrowing, literal wrapper typing, and body-based
// return inference (via the shared "js"-prefix return query).
// =============================================================================

use super::flow::JS_FLOW_CONFIG;
use super::JavascriptPlugin;
use crate::indexer::flow::run_flow_queries;
use crate::languages::LanguagePlugin;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};

fn js_grammar() -> tree_sitter::Language {
    JavascriptPlugin
        .grammar("javascript")
        .expect("JS grammar must load")
}

fn mk_sym(name: &str, kind: SymbolKind, start_line: u32) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind,
        visibility: None,
        start_line,
        end_line: start_line,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn mk_call_ref(target: &str, line: u32, byte_offset: u32) -> ExtractedRef {
    ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line,
        module: None,
        chain: None,
        byte_offset,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
        col: 0,
    }
}

#[test]
fn js_config_registered_on_plugin() {
    let cfg = JavascriptPlugin
        .flow_config()
        .expect("JS plugin must expose a FlowConfig");
    assert_eq!(cfg.strategy_prefix, "js");
    assert!(
        cfg.type_args_query.trim().is_empty(),
        "JS has no call-site type-argument syntax"
    );
}

/// The shared guard queries and the return query are written once for the
/// JS-family grammars; each must compile against the JavaScript grammar too.
#[test]
fn js_shared_queries_compile_against_js_grammar() {
    let grammar = js_grammar();
    for (name, src) in [
        ("assignment_query", JS_FLOW_CONFIG.assignment_query),
        ("type_guard_query", JS_FLOW_CONFIG.type_guard_query),
        (
            "discriminant_guard_query",
            JS_FLOW_CONFIG.discriminant_guard_query,
        ),
        (
            "return_query",
            crate::languages::typescript::flow::TS_RETURN_QUERY,
        ),
    ] {
        tree_sitter::Query::new(&grammar, src)
            .unwrap_or_else(|e| panic!("{name} failed to compile against JS grammar: {e:?}"));
    }
}

#[test]
fn js_flow_assignment_binds_lhs_to_rhs_ref() {
    let source = "const x = foo();\n";
    // 'const ' = 0..6, 'x' = 6, ' = ' = 7..10, 'foo' starts at byte 10.
    let symbols = vec![mk_sym("x", SymbolKind::Variable, 0)];
    let mut refs = vec![mk_call_ref("foo", 0, 10)];

    let meta = run_flow_queries(source, &js_grammar(), &JS_FLOW_CONFIG, &symbols, &mut refs);

    assert_eq!(
        meta.flow_binding_lhs.get(&0),
        Some(&0),
        "ref 0 (foo call) must bind to symbol 0 (x)"
    );
}

#[test]
fn js_flow_await_binding_flags_lhs() {
    let source = "const user = await getUser();\n";
    // 'const ' = 0..6, 'user' = 6..10, ' = ' = 10..13, 'await ' = 13..19,
    // 'getUser' starts at byte 19.
    let symbols = vec![mk_sym("user", SymbolKind::Variable, 0)];
    let mut refs = vec![mk_call_ref("getUser", 0, 19)];

    let meta = run_flow_queries(source, &js_grammar(), &JS_FLOW_CONFIG, &symbols, &mut refs);

    assert_eq!(
        meta.flow_binding_lhs.get(&0),
        Some(&0),
        "ref 0 (getUser call) must bind to symbol 0 (user)"
    );
    assert!(
        meta.flow_binding_await.contains(&0),
        "await initializer must flag the binding for async-wrapper peeling; meta={meta:?}"
    );
}

#[test]
fn js_flow_object_destructure_binds_each_field() {
    let source = "const { hits, total: count } = useAlgolia();\n";
    // 'hits' = 8..12, 'count' = 21..26, 'useAlgolia' starts at byte 31.
    let symbols = vec![
        mk_sym("hits", SymbolKind::Variable, 0),
        mk_sym("count", SymbolKind::Variable, 0),
    ];
    let mut refs = vec![mk_call_ref("useAlgolia", 0, 31)];

    let meta = run_flow_queries(source, &js_grammar(), &JS_FLOW_CONFIG, &symbols, &mut refs);

    let entries = meta
        .flow_binding_destructure
        .get(&0)
        .expect("RHS ref 0 (useAlgolia call) must carry destructure entries");
    assert!(
        entries.contains(&(0, "hits".to_string())),
        "shorthand `hits` must bind symbol 0 to field \"hits\"; got {entries:?}"
    );
    assert!(
        entries.contains(&(1, "total".to_string())),
        "renamed `total: count` must bind symbol 1 (count) to field \"total\"; got {entries:?}"
    );
}

#[test]
fn js_flow_object_destructure_await_marks_the_ref_awaited() {
    let source = "const { handle } = await fetchStatus();\n";
    // 'handle' = 8..14, 'await' = 19..24, 'fetchStatus' starts at byte 25.
    let symbols = vec![mk_sym("handle", SymbolKind::Variable, 0)];
    let mut refs = vec![mk_call_ref("fetchStatus", 0, 25)];

    let meta = run_flow_queries(source, &js_grammar(), &JS_FLOW_CONFIG, &symbols, &mut refs);

    assert!(
        meta.flow_binding_destructure.contains_key(&0),
        "RHS ref 0 (fetchStatus call) must carry a destructure entry; meta={meta:?}"
    );
    assert!(
        meta.flow_binding_destructure_await.contains(&0),
        "await destructure RHS must be flagged for async-wrapper peeling"
    );
}

#[test]
fn js_flow_instanceof_narrowing_records_scope() {
    let source = "if (x instanceof Foo) { x.run(); }\n";
    let meta = run_flow_queries(source, &js_grammar(), &JS_FLOW_CONFIG, &[], &mut []);

    assert_eq!(meta.narrowings.len(), 1, "meta={meta:?}");
    let n = &meta.narrowings[0];
    assert_eq!(n.name, "x");
    assert_eq!(n.narrowed_type, "Foo");
    assert!(n.byte_end > n.byte_start, "narrowed scope must be non-empty");
}

#[test]
fn js_flow_typeof_narrowing_strips_quotes() {
    let source = "if (typeof v === \"string\") { v.trim(); }\n";
    let meta = run_flow_queries(source, &js_grammar(), &JS_FLOW_CONFIG, &[], &mut []);

    assert_eq!(meta.narrowings.len(), 1, "meta={meta:?}");
    assert_eq!(meta.narrowings[0].name, "v");
    assert_eq!(meta.narrowings[0].narrowed_type, "string");
}

#[test]
fn js_flow_discriminant_guard_narrows_receiver() {
    let source = "if (shape.kind === \"circle\") { draw(shape); }\n";
    let meta = run_flow_queries(source, &js_grammar(), &JS_FLOW_CONFIG, &[], &mut []);

    assert_eq!(meta.discriminant_narrowings.len(), 1, "meta={meta:?}");
    let d = &meta.discriminant_narrowings[0];
    assert_eq!(d.name, "shape");
    assert_eq!(d.prop, "kind");
    assert_eq!(d.literal, "\"circle\"");
    assert!(!d.negate);
}

#[test]
fn js_flow_array_literal_seeds_wrapper_type() {
    let source = "const xs = [];\n";
    let symbols = vec![mk_sym("xs", SymbolKind::Variable, 0)];
    let mut refs = vec![];

    let meta = run_flow_queries(source, &js_grammar(), &JS_FLOW_CONFIG, &symbols, &mut refs);

    assert_eq!(
        meta.flow_binding_decl_type.get(&0).map(String::as_str),
        Some("Array"),
        "bare array literal must seed the Array wrapper type; meta={meta:?}"
    );
}

#[test]
fn js_flow_return_call_binds_owner_function() {
    let source = "function make() { return build(); }\n";
    // 'function ' = 0..9, 'make' = 9..13, 'build' starts at byte 25.
    let symbols = vec![mk_sym("make", SymbolKind::Function, 0)];
    let mut refs = vec![mk_call_ref("build", 0, 25)];

    let meta = run_flow_queries(source, &js_grammar(), &JS_FLOW_CONFIG, &symbols, &mut refs);

    assert_eq!(
        meta.flow_return_lhs.get(&0),
        Some(&0),
        "returned call ref must bind to its owning function; meta={meta:?}"
    );
}

#[test]
fn js_flow_object_literal_return_records_member_names() {
    let source = "function makeLogger() { return { info, error }; }\n";
    let symbols = vec![mk_sym("makeLogger", SymbolKind::Function, 0)];
    let mut refs = vec![];

    let meta = run_flow_queries(source, &js_grammar(), &JS_FLOW_CONFIG, &symbols, &mut refs);

    assert_eq!(
        meta.flow_return_object,
        vec![(0, vec!["info".to_string(), "error".to_string()])],
        "object-literal return must record its member names for {{fn}}$Ret synthesis"
    );
}
