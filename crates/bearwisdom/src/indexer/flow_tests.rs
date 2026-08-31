// =============================================================================
// indexer/flow_tests.rs — End-to-end tests for the shared flow-typing runner
// =============================================================================

use crate::indexer::flow::{run_flow_queries, FlowConfig};
use crate::languages::typescript::TypeScriptPlugin;
use crate::languages::LanguagePlugin;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};

/// Minimal FlowConfig mirroring `typescript::flow::TS_FLOW_CONFIG` — kept
/// here so the test doesn't depend on the env-gated plugin method.
const TS_TEST_FLOW: FlowConfig = FlowConfig {
    strategy_prefix: "ts",
    assignment_query: r#"
        (variable_declarator
            name: (identifier) @lhs
            type: (type_annotation (_) @type)?
            value: (_) @rhs)

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)

        (variable_declarator
            name: (object_pattern
                [(shorthand_property_identifier_pattern) @destruct.bind
                 (pair_pattern
                    key: (property_identifier) @destruct.key
                    value: (identifier) @destruct.bind)])
            value: (_) @rhs)
    "#,
    type_guard_query: r#"
        (if_statement
            condition: (parenthesized_expression
                (binary_expression
                    left: (identifier) @guard.local
                    right: (identifier) @guard.type))
            consequence: (statement_block) @guard.body)
    "#,
    discriminant_guard_query: r#"
        (if_statement
            condition: (parenthesized_expression
                (binary_expression
                    left: (member_expression
                        object: (identifier) @guard.local
                        property: (property_identifier) @guard.prop)
                    operator: ["===" "=="]
                    right: (string) @guard.literal))
            consequence: (statement_block) @guard.body)

        (switch_statement
            value: (parenthesized_expression
                (member_expression
                    object: (identifier) @guard.local
                    property: (property_identifier) @guard.prop))
            body: (switch_body
                (switch_case
                    value: (string) @guard.literal) @guard.body))
    "#,
    type_args_query: r#"
        (call_expression
            function: (member_expression
                property: (property_identifier) @call.method)
            type_arguments: (type_arguments
                (type_identifier) @call.type_arg))

        (call_expression
            function: (identifier) @call.method
            type_arguments: (type_arguments
                (_) @call.type_arg))
    "#,
    literal_type_kinds: &[
        ("array", "Array"),
        ("object", "Object"),
        ("string", "String"),
        ("template_string", "String"),
        ("number", "Number"),
        ("regex", "RegExp"),
    ],
};

fn ts_grammar() -> tree_sitter::Language {
    TypeScriptPlugin
        .grammar("typescript")
        .expect("TS grammar must load")
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

fn mk_instantiates_ref(target: &str, line: u32, byte_offset: u32) -> ExtractedRef {
    ExtractedRef {
        kind: EdgeKind::Instantiates,
        ..mk_call_ref(target, line, byte_offset)
    }
}

#[test]
fn flow_assignment_binds_lhs_to_rhs_ref() {
    let source = "const x = foo();\n";
    // "const x = foo();" byte positions:
    //   'const ' = 0..6, 'x' = 6, ' = ' = 7..10, 'foo' = 10..13, '()' = 13..15
    let symbols = vec![mk_sym("x", SymbolKind::Variable, 0)];
    let mut refs = vec![
        // Ref for `foo()` call at byte offset 10 (start of `foo`).
        mk_call_ref("foo", 0, 10),
    ];

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    assert_eq!(
        meta.flow_binding_lhs.get(&0),
        Some(&0),
        "flow runner should bind ref 0 (foo call) to symbol 0 (x)"
    );
}

#[test]
fn flow_object_destructure_binds_each_field() {
    // `const { hits, total: count } = useAlgolia();` — each destructured binding
    // maps the RHS ref to its (symbol, source-field): shorthand `hits` → field
    // "hits"; renamed `total: count` → binding `count`, field "total".
    let source = "const { hits, total: count } = useAlgolia();\n";
    // byte positions: 'hits' = 8, 'count' = 21, 'useAlgolia' = 31, '()' = 41..43
    let symbols = vec![
        mk_sym("hits", SymbolKind::Variable, 0),
        mk_sym("count", SymbolKind::Variable, 0),
    ];
    let mut refs = vec![mk_call_ref("useAlgolia", 0, 31)];

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

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
fn flow_object_destructure_await_marks_the_ref_awaited() {
    // `const { handle } = await fetchStatus();` — the destructure RHS is an
    // `await_expression` wrapping the call; the runner must flag the RHS ref
    // as awaited so the resolver strips the async wrapper before projecting
    // `handle`.
    let source = "const { handle } = await fetchStatus();\n";
    // byte positions: 'handle' = 8, 'await' = 19, 'fetchStatus' = 25
    let symbols = vec![mk_sym("handle", SymbolKind::Variable, 0)];
    let mut refs = vec![mk_call_ref("fetchStatus", 0, 25)];

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    assert!(
        meta.flow_binding_destructure.contains_key(&0),
        "RHS ref 0 (fetchStatus call) must carry a destructure entry; meta={meta:?}"
    );
    assert!(
        meta.flow_binding_destructure_await.contains(&0),
        "RHS ref 0 must be flagged awaited; meta={meta:?}"
    );
}

#[test]
fn flow_nested_construction_binds_outer_constructor() {
    // `const x = new Outer(new Inner());` — two chain-less Instantiates refs.
    // The outer constructor is the initializer's type, so the lhs must bind to
    // the leftmost (outermost) ref, not the furthest-right nested one.
    let source = "const x = new Outer(new Inner());\n";
    //   'const x = ' = 0..10, 'new ' = 10..14, 'Outer' = 14..19,
    //   '(new ' = 19..24, 'Inner' = 24..29
    let symbols = vec![mk_sym("x", SymbolKind::Variable, 0)];
    let mut refs = vec![
        mk_instantiates_ref("Outer", 0, 14),
        mk_instantiates_ref("Inner", 0, 24),
    ];

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    assert_eq!(
        meta.flow_binding_lhs.get(&0),
        Some(&0),
        "lhs `x` must bind to ref 0 (outer `new Outer`), not the nested `new Inner`"
    );
    assert!(
        !meta.flow_binding_lhs.contains_key(&1),
        "the nested `new Inner` ref must not be the binding initializer"
    );
}

#[test]
fn flow_reassignment_also_binds() {
    let source = "let x = 1;\nx = foo();\n";
    // Byte positions:
    //   'let x = 1;' = 0..10
    //   '\n' = 10
    //   'x = foo();' = 11..21  (x at 11, foo at 15)
    let symbols = vec![mk_sym("x", SymbolKind::Variable, 0)];
    let mut refs = vec![
        // foo() at byte 15
        mk_call_ref("foo", 1, 15),
    ];

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    assert_eq!(
        meta.flow_binding_lhs.get(&0),
        Some(&0),
        "reassignment should also bind ref 0 to x"
    );
}

#[test]
fn flow_return_binds_call_to_function() {
    // function makeUser() { return build(); }
    // `build` starts at byte 31 (line 1).
    let source = "function makeUser() {\n  return build();\n}\n";
    let symbols = vec![mk_sym("makeUser", SymbolKind::Function, 0)];
    let mut refs = vec![mk_call_ref("build", 1, 31)];

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    assert_eq!(
        meta.flow_return_lhs.get(&0),
        Some(&0),
        "the return expression ref (build call) should bind to function symbol 0 (makeUser)"
    );
}

#[test]
fn flow_return_bare_identifier_records_ident() {
    // function f(x) { return x; }  — `x` is a bare param read, no ref. The
    // ref-based flow_return_lhs misses it; flow_return_ident records (fn, "x")
    // so the resolver types x against f's parameters and harvests its return.
    let source = "function f(x) {\n  return x;\n}\n";
    let symbols = vec![mk_sym("f", SymbolKind::Function, 0)];
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    assert!(
        meta.flow_return_lhs.is_empty(),
        "a bare identifier return carries no ref"
    );
    assert_eq!(
        meta.flow_return_ident,
        vec![(0usize, "x".to_string())],
        "bare identifier return should record (fn_idx, ident)"
    );
}

#[test]
fn flow_return_ignores_nested_callback_return() {
    // function f() { items.forEach(x => { return g(); }); }
    // The inner arrow's `return g()` is NOT a direct child of f's body block,
    // so it must not be attributed to f (soundness: no nested-scope leakage).
    // `g` starts at byte 45 (line 1).
    let source = "function f() {\n  items.forEach(x => { return g(); });\n}\n";
    let symbols = vec![mk_sym("f", SymbolKind::Function, 0)];
    let mut refs = vec![mk_call_ref("g", 1, 45)];

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    assert!(
        meta.flow_return_lhs.is_empty(),
        "a return inside a nested callback must not bind to the enclosing function"
    );
}

#[test]
fn flow_return_captures_nested_if_return() {
    // function makeUser(c: boolean) { if (c) { return build(); } }
    // The `return build()` is nested in an if-block, not a direct child of the
    // function body. The widened descendant query captures it, and the
    // ancestor-walk attributes it to makeUser (its nearest enclosing function).
    let source = "function makeUser(c: boolean) {\n  if (c) {\n    return build();\n  }\n}\n";
    let build_off = source.find("build()").unwrap() as u32;
    let symbols = vec![mk_sym("makeUser", SymbolKind::Function, 0)];
    let mut refs = vec![mk_call_ref("build", 2, build_off)];

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    assert_eq!(
        meta.flow_return_lhs.get(&0),
        Some(&0),
        "a return nested in an if-block binds to the enclosing function makeUser"
    );
}

#[test]
fn flow_return_captures_arrow_const_body() {
    // const makeUser = () => { return build(); };
    // The arrow-const body's return must attribute to the `makeUser` symbol
    // (emitted as a Function at the declarator row).
    let source = "const makeUser = () => {\n  return build();\n};\n";
    let build_off = source.find("build()").unwrap() as u32;
    let symbols = vec![mk_sym("makeUser", SymbolKind::Function, 0)];
    let mut refs = vec![mk_call_ref("build", 1, build_off)];

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    assert_eq!(
        meta.flow_return_lhs.get(&0),
        Some(&0),
        "an arrow-const body return binds to the const symbol makeUser"
    );
}

#[test]
fn flow_return_nested_callback_still_ignored() {
    // function f() { if (true) { items.forEach(x => { return g(); }); } }
    // The inner arrow's `return g()` is now reachable by descendant matching,
    // but its nearest enclosing function is the arrow, not f — the ancestor-walk
    // guard must still reject it.
    let source =
        "function f() {\n  if (true) {\n    items.forEach(x => { return g(); });\n  }\n}\n";
    let g_off = source.find("g()").unwrap() as u32;
    let symbols = vec![mk_sym("f", SymbolKind::Function, 0)];
    let mut refs = vec![mk_call_ref("g", 2, g_off)];

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    assert!(
        meta.flow_return_lhs.is_empty(),
        "a callback return reachable by descendant matching must still be rejected by the ancestor-walk"
    );
}

#[test]
fn flow_narrowing_captures_instanceof_body() {
    let source = "function f(x: Base) {\n  if (x instanceof Derived) {\n    x.foo();\n  }\n}\n";
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    assert!(
        !meta.narrowings.is_empty(),
        "type-guard query should capture at least one narrowing"
    );
    let n = &meta.narrowings[0];
    assert_eq!(n.name, "x");
    assert_eq!(n.narrowed_type, "Derived");
    assert!(n.byte_end > n.byte_start);
}

#[test]
fn flow_reassignment_kills_narrowing_for_rest_of_scope() {
    // A reassignment to `x` inside a narrowed block invalidates the narrowing
    // from that point on — `x.bar()` after `x = reset()` must NOT see `Derived`.
    // The narrowing's range is truncated to end at the reassignment.
    let source =
        "function f(x: Base) {\n  if (x instanceof Derived) {\n    x.foo();\n    x = reset();\n    x.bar();\n  }\n}\n";
    let symbols = vec![mk_sym("x", SymbolKind::Variable, 0)];
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    let n = meta
        .narrowings
        .iter()
        .find(|n| n.name == "x")
        .expect("instanceof guard should narrow `x`");
    assert_eq!(n.narrowed_type, "Derived");

    // The reassignment `x = reset();` sits before `x.bar();`. The narrowing
    // must end at or before `x.bar()` so the later use is not narrowed.
    let reassign_pos = source.find("x = reset()").unwrap() as u32;
    let bar_pos = source.find("x.bar()").unwrap() as u32;
    assert!(
        n.byte_end <= bar_pos,
        "narrowing range [{}, {}) must not cover `x.bar()` at {bar_pos} (killed by reassignment at {reassign_pos})",
        n.byte_start,
        n.byte_end
    );
    // The use before the reassignment (`x.foo()`) is still narrowed.
    let foo_pos = source.find("x.foo()").unwrap() as u32;
    assert!(
        n.byte_start <= foo_pos && foo_pos < n.byte_end,
        "narrowing range [{}, {}) should still cover `x.foo()` at {foo_pos}",
        n.byte_start,
        n.byte_end
    );
}

#[test]
fn flow_discriminant_guard_captures_prop_and_literal() {
    let source = "function f(s: Shape) {\n  if (s.kind === \"circle\") {\n    s.radius;\n  }\n}\n";
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    assert_eq!(
        meta.discriminant_narrowings.len(),
        1,
        "discriminant-guard query should capture one narrowing"
    );
    let d = &meta.discriminant_narrowings[0];
    assert_eq!(d.name, "s");
    assert_eq!(d.prop, "kind");
    assert_eq!(d.literal, "\"circle\"");
    assert!(d.byte_end > d.byte_start);
}

#[test]
fn flow_inequality_guard_is_not_a_discriminant() {
    // `!==` narrows the else-branch, not the consequent — must not be captured.
    let source = "function f(s: Shape) {\n  if (s.kind !== \"circle\") {\n    return;\n  }\n}\n";
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();
    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);
    assert!(meta.discriminant_narrowings.is_empty());
}

#[test]
fn flow_discriminant_switch_captures_each_case() {
    let source = "function f(s: Shape) {\n  switch (s.kind) {\n    case \"circle\": s.radius; break;\n    case \"square\": s.side; break;\n  }\n}\n";
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();
    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);
    assert_eq!(
        meta.discriminant_narrowings.len(),
        2,
        "one narrowing per case clause"
    );
    let circle = meta
        .discriminant_narrowings
        .iter()
        .find(|d| d.literal == "\"circle\"")
        .expect("circle case");
    assert_eq!(circle.name, "s");
    assert_eq!(circle.prop, "kind");
    assert!(meta
        .discriminant_narrowings
        .iter()
        .any(|d| d.name == "s" && d.prop == "kind" && d.literal == "\"square\""));
}

#[test]
fn flow_type_args_populate_chain_segment() {
    use crate::types::{ChainSegment, MemberChain, SegmentKind};

    let source = "repo.findOne<User>();\n";
    // Byte positions:
    //   'repo' = 0..4
    //   '.findOne' = 4..12
    //   findOne at bytes 5..12 (property_identifier: 'findOne')
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs = vec![ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: "findOne".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: Some(MemberChain {
            segments: vec![
                ChainSegment {
                    name: "repo".to_string(),
                    node_kind: "identifier".to_string(),
                    kind: SegmentKind::Identifier,
                    declared_type: None,
                    type_args: Vec::new(),
                    optional_chaining: false,
                    byte_offset: 0,
                    declared_type_id: None,
                    is_call: false,
                    call_args: Vec::new(),
                    type_arg_ids: Vec::new(),
                },
                ChainSegment {
                    name: "findOne".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: Vec::new(),
                    optional_chaining: false,
                    byte_offset: 0,
                    declared_type_id: None,
                    is_call: false,
                    call_args: Vec::new(),
                    type_arg_ids: Vec::new(),
                },
            ],
        }),
        byte_offset: 5, // inside the `findOne` span (5..12)
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }];

    let _ = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    let segs = &refs[0].chain.as_ref().unwrap().segments;
    let last = segs.last().unwrap();
    assert_eq!(
        last.type_args,
        vec!["User".to_string()],
        "type-args query should populate the chain segment's type_args"
    );
}

#[test]
fn flow_bare_call_type_args_populate_segment() {
    use crate::types::{ChainSegment, MemberChain, SegmentKind};

    // `useQuery<DogsResp>()` — a bare (non-member) generic call. Its single chain
    // segment must receive the type arg so the resolver can bind it into the
    // callee's return.
    let source = "const x = useQuery<DogsResp>();\n";
    //   'const x = ' = 0..10, 'useQuery' = 10..18, '<DogsResp>' = 18..28
    let symbols: Vec<ExtractedSymbol> = vec![mk_sym("x", SymbolKind::Variable, 0)];
    let mut refs = vec![ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: "useQuery".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        chain: Some(MemberChain {
            segments: vec![ChainSegment {
                name: "useQuery".to_string(),
                node_kind: "identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            }],
        }),
        byte_offset: 10, // start of `useQuery`
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }];

    let _ = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    let last = refs[0].chain.as_ref().unwrap().segments.last().unwrap();
    assert_eq!(
        last.type_args,
        vec!["DogsResp".to_string()],
        "bare-call type-args query should populate the chain segment's type_args"
    );
}

#[test]
fn rust_let_mut_annotation_records_declared_type() {
    use crate::languages::rust_lang::flow::RUST_FLOW_CONFIG;
    use crate::languages::rust_lang::RustLangPlugin;

    // `let mut x: T = …` — pattern is a `mut_pattern`, type is the annotation.
    // The runner must capture the declared type for the bare-identifier name
    // even though the initializer (`.unwrap()`) wouldn't resolve to it.
    let source = "fn f() {\n    let mut index_writer: IndexWriter = build().unwrap();\n}\n";
    let grammar = RustLangPlugin
        .grammar("rust")
        .expect("rust grammar must load");
    let symbols = vec![mk_sym("index_writer", SymbolKind::Variable, 1)];
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &grammar, &RUST_FLOW_CONFIG, &symbols, &mut refs);

    assert_eq!(
        meta.flow_binding_decl_type.get(&0),
        Some(&"IndexWriter".to_string()),
        "the `let mut x: IndexWriter` annotation types symbol 0 directly"
    );
}

#[test]
fn rust_generic_annotation_records_full_type() {
    use crate::languages::rust_lang::flow::RUST_FLOW_CONFIG;
    use crate::languages::rust_lang::RustLangPlugin;

    // Generic annotation `Vec<String>` records the full text so the type
    // interner can decompose it into `Apply(Vec, [String])` — the head still
    // keys the member lookup, and the element type survives for subscript
    // projection (`names[0]`).
    let source = "fn f() {\n    let names: Vec<String> = make();\n}\n";
    let grammar = RustLangPlugin
        .grammar("rust")
        .expect("rust grammar must load");
    let symbols = vec![mk_sym("names", SymbolKind::Variable, 1)];
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &grammar, &RUST_FLOW_CONFIG, &symbols, &mut refs);

    assert_eq!(
        meta.flow_binding_decl_type.get(&0),
        Some(&"Vec<String>".to_string()),
        "generic annotation records the full declared type text"
    );
}

#[test]
fn ts_function_parameter_annotation_records_declared_type() {
    use crate::languages::typescript::flow::TS_FLOW_CONFIG;

    // A function parameter `text: string` must seed the flow cache so a member
    // call on the param (`text.replace(...)`) types the receiver `string` and
    // routes to `String`'s member index — not a foreign same-named binding. The
    // param is neither a `variable_declarator` nor an `assignment_expression`, so
    // it needs its own capture in the assignment query.
    let source =
        "function toKebabCase(text: string): string {\n    return text.replace(/a/, \"b\");\n}\n";
    let grammar = TypeScriptPlugin
        .grammar("typescript")
        .expect("typescript grammar must load");
    let symbols = vec![mk_sym("text", SymbolKind::Property, 0)];
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &grammar, &TS_FLOW_CONFIG, &symbols, &mut refs);

    assert_eq!(
        meta.flow_binding_decl_type.get(&0),
        Some(&"string".to_string()),
        "the `text: string` parameter annotation types symbol 0 directly"
    );
}

#[test]
fn csharp_declaration_pattern_narrows_binding() {
    use crate::languages::csharp::CSharpPlugin;

    // `if (user is Admin admin) { admin.Ban(); }` — the binding `admin` is typed
    // Admin within the block (C# declaration pattern).
    let source = "class C {\n  void M(object user) {\n    if (user is Admin admin) {\n      admin.Ban();\n    }\n  }\n}\n";
    let grammar = CSharpPlugin
        .grammar("csharp")
        .expect("c# grammar must load");
    let cfg = CSharpPlugin.flow_config().expect("c# flow config");
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &grammar, cfg, &symbols, &mut refs);

    let n = meta
        .narrowings
        .iter()
        .find(|n| n.name == "admin")
        .expect("declaration-pattern binding `admin` should narrow");
    assert_eq!(n.narrowed_type, "Admin");
    assert!(n.byte_end > n.byte_start);
}

#[test]
fn rust_try_operator_marks_binding_for_unwrap() {
    use crate::languages::rust_lang::flow::RUST_FLOW_CONFIG;
    use crate::languages::rust_lang::RustLangPlugin;

    // `let reader = index.reader()?;` — the `?` makes the RHS a try_expression.
    // The runner binds the inner call ref to the LHS AND flags the binding so
    // the resolver peels `Result<IndexReader>` → `IndexReader`.
    let source = "fn f() {\n    let reader = index.reader()?;\n}\n";
    let grammar = RustLangPlugin
        .grammar("rust")
        .expect("rust grammar must load");
    let symbols = vec![mk_sym("reader", SymbolKind::Variable, 1)];
    // A Calls ref for `index.reader()` — byte offset inside the try_expression.
    // "fn f() {\n    let reader = " is 26 bytes; `index.reader()?` starts at 26,
    // the `reader` call segment lands a few bytes in.
    let mut refs = vec![mk_call_ref("reader", 1, 32)];

    let meta = run_flow_queries(source, &grammar, &RUST_FLOW_CONFIG, &symbols, &mut refs);

    assert_eq!(
        meta.flow_binding_lhs.get(&0),
        Some(&0),
        "the call ref inside the try-expression binds to `reader`"
    );
    assert!(
        meta.flow_binding_unwrap.contains(&0),
        "the `?` flags symbol 0 (`reader`) for wrapper peeling"
    );
}

#[test]
fn java_instanceof_pattern_binding_narrows() {
    use crate::languages::java::JavaPlugin;

    // `if (x instanceof Admin a) { a.ban(); }` — Java 16+ pattern binding types
    // `a` as Admin; the bindingless form additionally narrows the receiver `x`.
    let source = "class C {\n  void m(Object x) {\n    if (x instanceof Admin a) {\n      a.ban();\n    }\n  }\n}\n";
    let grammar = JavaPlugin.grammar("java").expect("java grammar must load");
    let cfg = JavaPlugin.flow_config().expect("java flow config");
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &grammar, cfg, &symbols, &mut refs);

    assert!(
        meta.narrowings
            .iter()
            .any(|n| n.name == "a" && n.narrowed_type == "Admin"),
        "pattern binding `a` should narrow to Admin; got {:?}",
        meta.narrowings
    );
    assert!(
        meta.narrowings
            .iter()
            .any(|n| n.name == "x" && n.narrowed_type == "Admin"),
        "the receiver `x` also narrows to Admin"
    );
}

#[test]
fn ruby_kind_of_narrows_like_is_a() {
    use crate::languages::ruby::RubyPlugin;

    // `if x.kind_of?(Foo) then ... end` — the `kind_of?` alias narrows like `is_a?`.
    let source = "if x.kind_of?(Foo)\n  x.bar\nend\n";
    let grammar = RubyPlugin.grammar("ruby").expect("ruby grammar must load");
    let cfg = RubyPlugin.flow_config().expect("ruby flow config");
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &grammar, cfg, &symbols, &mut refs);

    assert!(
        meta.narrowings
            .iter()
            .any(|n| n.name == "x" && n.narrowed_type == "Foo"),
        "kind_of? should narrow `x` to Foo; got {:?}",
        meta.narrowings
    );
}

#[test]
fn go_type_switch_narrows_alias_per_case() {
    use crate::languages::go::flow::GO_FLOW_CONFIG;
    use crate::languages::go::GoPlugin;

    // `switch v := x.(type) { case *Admin: v.Ban() }` narrows `v` to Admin in
    // the matching case body. `flow_config()` is disabled for Go (go-pocketbase
    // OOM), so the static is referenced directly to validate the query.
    let source =
        "func f(x interface{}) {\n\tswitch v := x.(type) {\n\tcase *Admin:\n\t\tv.Ban()\n\t}\n}\n";
    let grammar = GoPlugin.grammar("go").expect("go grammar must load");
    let cfg = &GO_FLOW_CONFIG;
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &grammar, cfg, &symbols, &mut refs);

    assert!(
        meta.narrowings
            .iter()
            .any(|n| n.name == "v" && n.narrowed_type == "Admin"),
        "type switch should narrow `v` to Admin; got {:?}",
        meta.narrowings
    );
}

#[test]
fn ts_typeof_string_guard_narrows() {
    // `if (typeof x === "string") { ... }` narrows `x` to the string primitive.
    use crate::languages::typescript::flow::TS_FLOW_CONFIG;
    let source =
        "function f(x: unknown) {\n  if (typeof x === \"string\") {\n    x.length;\n  }\n}\n";
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &ts_grammar(), &TS_FLOW_CONFIG, &symbols, &mut refs);

    assert!(
        meta.narrowings
            .iter()
            .any(|n| n.name == "x" && n.narrowed_type == "string"),
        "typeof guard should narrow `x` to string; got {:?}",
        meta.narrowings
    );
}

#[test]
fn flow_early_return_guard_negates_and_scopes_after_block() {
    use crate::languages::typescript::flow::TS_FLOW_CONFIG;
    // `if (s.kind !== "circle") return;` — `s` narrows to NOT-circle for the
    // rest of the enclosing block (the early `return` makes the negation hold).
    let source = "function f(s: Shape) {\n  if (s.kind !== \"circle\") return;\n  s.radius;\n}\n";
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();
    let meta = run_flow_queries(source, &ts_grammar(), &TS_FLOW_CONFIG, &symbols, &mut refs);

    let d = meta
        .discriminant_narrowings
        .iter()
        .find(|d| d.name == "s" && d.negate)
        .expect("negated early-return guard should produce a narrowing");
    assert_eq!(d.prop, "kind");
    assert_eq!(d.literal, "\"circle\"");
    // The narrowing scopes AFTER the guard — `s.radius;` falls inside its range.
    let radius_pos = source.find("s.radius").unwrap() as u32;
    assert!(
        d.byte_start <= radius_pos && radius_pos < d.byte_end,
        "range [{}, {}) should cover s.radius at {radius_pos}",
        d.byte_start,
        d.byte_end
    );
}

/// Per-language nested-if return smoke. Builds a return-only FlowConfig with
/// `strategy_prefix` so `run_return_query` picks the language's RETURN_QUERY,
/// parses a snippet whose `return build()` is nested in an if-block, and asserts
/// the `build` ref binds to the enclosing function symbol via the ancestor-walk.
/// `fn_kind` is the symbol kind the extractor would emit for the enclosing
/// function (Method inside a class, Function for a free function).
fn assert_nested_if_return_binds(
    prefix: &'static str,
    grammar: &tree_sitter::Language,
    src: &str,
    fn_name: &str,
    fn_line: u32,
    fn_kind: SymbolKind,
) {
    let cfg = FlowConfig {
        strategy_prefix: prefix,
        assignment_query: "",
        type_guard_query: "",
        discriminant_guard_query: "",
        type_args_query: "",
        literal_type_kinds: &[],
    };
    let build_off = src.find("build").unwrap() as u32;
    let build_line = src[..build_off as usize].matches('\n').count() as u32;
    let symbols = vec![mk_sym(fn_name, fn_kind, fn_line)];
    let mut refs = vec![mk_call_ref("build", build_line, build_off)];

    let meta = run_flow_queries(src, grammar, &cfg, &symbols, &mut refs);

    assert_eq!(
        meta.flow_return_lhs.get(&0),
        Some(&0),
        "{prefix}: nested-if `return build()` should bind to {fn_name} (symbol 0)"
    );
}

#[test]
fn flow_return_python_nested_if() {
    use crate::languages::python::PythonPlugin;
    let g = PythonPlugin.grammar("python").unwrap();
    assert_nested_if_return_binds(
        "python",
        &g,
        "def make_user(c):\n    if c:\n        return build()\n",
        "make_user",
        0,
        SymbolKind::Function,
    );
}

#[test]
fn flow_return_go_nested_if() {
    use crate::languages::go::GoPlugin;
    let g = GoPlugin.grammar("go").unwrap();
    assert_nested_if_return_binds(
        "go",
        &g,
        "func makeUser(c bool) T {\n\tif c {\n\t\treturn build()\n\t}\n}\n",
        "makeUser",
        0,
        SymbolKind::Function,
    );
}

#[test]
fn flow_return_java_nested_if() {
    use crate::languages::java::JavaPlugin;
    let g = JavaPlugin.grammar("java").unwrap();
    assert_nested_if_return_binds(
        "java",
        &g,
        "class C {\n  T makeUser(boolean c) {\n    if (c) {\n      return build();\n    }\n  }\n}\n",
        "makeUser",
        1,
        SymbolKind::Method,
    );
}

#[test]
fn flow_return_rust_nested_if() {
    use crate::languages::rust_lang::RustLangPlugin;
    let g = RustLangPlugin.grammar("rust").unwrap();
    assert_nested_if_return_binds(
        "rust",
        &g,
        "fn make_user(c: bool) -> T {\n    if c {\n        return build();\n    }\n}\n",
        "make_user",
        0,
        SymbolKind::Function,
    );
}

#[test]
fn flow_return_csharp_nested_if() {
    use crate::languages::csharp::CSharpPlugin;
    let g = CSharpPlugin.grammar("csharp").unwrap();
    assert_nested_if_return_binds(
        "csharp",
        &g,
        "class C {\n  T MakeUser(bool c) {\n    if (c) {\n      return build();\n    }\n  }\n}\n",
        "MakeUser",
        1,
        SymbolKind::Method,
    );
}

#[test]
fn flow_return_kotlin_nested_if() {
    use crate::languages::kotlin::KotlinPlugin;
    let g = KotlinPlugin.grammar("kotlin").unwrap();
    assert_nested_if_return_binds(
        "kotlin",
        &g,
        "fun makeUser(c: Boolean): T {\n    if (c) {\n        return build()\n    }\n}\n",
        "makeUser",
        0,
        SymbolKind::Function,
    );
}

#[test]
fn flow_return_php_nested_if() {
    use crate::languages::php::PhpPlugin;
    let g = PhpPlugin.grammar("php").unwrap();
    assert_nested_if_return_binds(
        "php",
        &g,
        "<?php\nfunction makeUser($c) {\n    if ($c) {\n        return build();\n    }\n}\n",
        "makeUser",
        1,
        SymbolKind::Function,
    );
}

#[test]
fn flow_return_scala_nested_if() {
    use crate::languages::scala::ScalaPlugin;
    let g = ScalaPlugin.grammar("scala").unwrap();
    assert_nested_if_return_binds(
        "scala",
        &g,
        "def makeUser(c: Boolean): T = {\n  if (c) {\n    return build()\n  }\n}\n",
        "makeUser",
        0,
        SymbolKind::Function,
    );
}

#[test]
fn flow_return_ruby_nested_if() {
    use crate::languages::ruby::RubyPlugin;
    let g = RubyPlugin.grammar("ruby").unwrap();
    assert_nested_if_return_binds(
        "ruby",
        &g,
        "def make_user(c)\n  if c\n    return build()\n  end\nend\n",
        "make_user",
        0,
        SymbolKind::Method,
    );
}

/// Concise expression-body return: a function whose body is a single
/// expression with no `return` keyword (TS `() => expr`, Scala/Kotlin `= expr`).
/// The `@return.tail` query arm captures the body expression and the same
/// ancestor-walk + name correlation attributes it to the owning function.
fn assert_concise_body_return_binds(
    prefix: &'static str,
    grammar: &tree_sitter::Language,
    src: &str,
    fn_name: &str,
    fn_line: u32,
    fn_kind: SymbolKind,
) {
    let cfg = FlowConfig {
        strategy_prefix: prefix,
        assignment_query: "",
        type_guard_query: "",
        discriminant_guard_query: "",
        type_args_query: "",
        literal_type_kinds: &[],
    };
    let build_off = src.find("build").unwrap() as u32;
    let build_line = src[..build_off as usize].matches('\n').count() as u32;
    let symbols = vec![mk_sym(fn_name, fn_kind, fn_line)];
    let mut refs = vec![mk_call_ref("build", build_line, build_off)];

    let meta = run_flow_queries(src, grammar, &cfg, &symbols, &mut refs);

    assert_eq!(
        meta.flow_return_lhs.get(&0),
        Some(&0),
        "{prefix}: concise-body `build()` should bind to {fn_name} (symbol 0)"
    );
}

#[test]
fn flow_return_ts_arrow_concise_body() {
    // const makeUser = () => build();  — no `return`, body is a bare call.
    assert_concise_body_return_binds(
        "ts",
        &ts_grammar(),
        "const makeUser = () => build();\n",
        "makeUser",
        0,
        SymbolKind::Function,
    );
}

#[test]
fn flow_return_scala_concise_body() {
    // def makeUser = build()  — Scala `=`-body with no block, no `return`.
    use crate::languages::scala::ScalaPlugin;
    let g = ScalaPlugin.grammar("scala").unwrap();
    assert_concise_body_return_binds(
        "scala",
        &g,
        "object O {\n  def makeUser = build()\n}\n",
        "makeUser",
        1,
        SymbolKind::Function,
    );
}

#[test]
fn flow_return_kotlin_concise_body() {
    // fun makeUser() = build()  — Kotlin expression body, no block, no `return`.
    use crate::languages::kotlin::KotlinPlugin;
    let g = KotlinPlugin.grammar("kotlin").unwrap();
    assert_concise_body_return_binds(
        "kotlin",
        &g,
        "fun makeUser() = build()\n",
        "makeUser",
        0,
        SymbolKind::Function,
    );
}

#[test]
fn flow_return_ts_block_body_tail_is_not_captured() {
    // const f = () => { return build(); };  — the arrow body is a statement_block,
    // so the `@return.tail` arm must NOT fire on it (the block is excluded by
    // `block_kinds`); the explicit `return` inside is still captured by
    // `@return.expr`. Either way `build` binds to `f` exactly once — assert the
    // bind exists without a spurious second attribution.
    let cfg = FlowConfig {
        strategy_prefix: "ts",
        assignment_query: "",
        type_guard_query: "",
        discriminant_guard_query: "",
        type_args_query: "",
        literal_type_kinds: &[],
    };
    let src = "const f = () => {\n  return build();\n};\n";
    let build_off = src.find("build").unwrap() as u32;
    let symbols = vec![mk_sym("f", SymbolKind::Function, 0)];
    let mut refs = vec![mk_call_ref("build", 1, build_off)];
    let meta = run_flow_queries(src, &ts_grammar(), &cfg, &symbols, &mut refs);
    assert_eq!(
        meta.flow_return_lhs.get(&0),
        Some(&0),
        "the explicit return inside the block binds to f; the block-as-tail must not double-fire"
    );
}

/// Soundness across languages: a `return` inside a nested lambda must NOT bind
/// to the enclosing named function (its nearest function ancestor is the
/// lambda, which carries no symbol here). Mirrors the TS callback test for the
/// languages whose grammar models an inline lambda with a `return`.
#[test]
fn flow_return_nested_lambda_not_attributed_cross_lang() {
    fn assert_lambda_return_ignored(
        prefix: &'static str,
        grammar: &tree_sitter::Language,
        src: &str,
        fn_name: &str,
    ) {
        let cfg = FlowConfig {
            strategy_prefix: prefix,
            assignment_query: "",
            type_guard_query: "",
            discriminant_guard_query: "",
            type_args_query: "",
            literal_type_kinds: &[],
        };
        let g_off = src.find("inner(").unwrap() as u32;
        let g_line = src[..g_off as usize].matches('\n').count() as u32;
        let symbols = vec![mk_sym(fn_name, SymbolKind::Method, 0)];
        let mut refs = vec![mk_call_ref("inner", g_line, g_off)];
        let meta = run_flow_queries(src, grammar, &cfg, &symbols, &mut refs);
        assert!(
            meta.flow_return_lhs.is_empty(),
            "{prefix}: a return inside a nested lambda must not bind to {fn_name}; got {:?}",
            meta.flow_return_lhs
        );
    }

    use crate::languages::java::JavaPlugin;
    // Java: the lambda body `() -> { return inner(); }` is a `lambda_expression`
    // (a function_kind); its return resolves to the anonymous lambda, not `m`.
    let jg = JavaPlugin.grammar("java").unwrap();
    assert_lambda_return_ignored(
        "java",
        &jg,
        "class C {\n  T m() {\n    run(() -> { return inner(); });\n  }\n}\n",
        "m",
    );

    use crate::languages::php::PhpPlugin;
    // PHP: the callback `function () { return inner(); }` is an
    // `anonymous_function`; its return must resolve to the closure, not `m`.
    let pg = PhpPlugin.grammar("php").unwrap();
    assert_lambda_return_ignored(
        "php",
        &pg,
        "<?php\nfunction m() {\n    array_map(function () { return inner(); }, $xs);\n}\n",
        "m",
    );

    use crate::languages::kotlin::KotlinPlugin;
    // Kotlin: the trailing `run { return inner() }` body is a `lambda_literal`;
    // the return must resolve to the lambda, not `m`.
    let kg = KotlinPlugin.grammar("kotlin").unwrap();
    assert_lambda_return_ignored(
        "kotlin",
        &kg,
        "fun m() {\n    run { return inner() }\n}\n",
        "m",
    );

    use crate::languages::scala::ScalaPlugin;
    // Scala: the `x => { return inner() }` argument is a `lambda_expression`;
    // the return must resolve to the lambda, not `m`.
    let sg = ScalaPlugin.grammar("scala").unwrap();
    assert_lambda_return_ignored(
        "scala",
        &sg,
        "object O {\n  def m() = {\n    xs.foreach(x => { return inner() })\n  }\n}\n",
        "m",
    );

    use crate::languages::ruby::RubyPlugin;
    // Ruby: the `do ... end` callback is a `do_block`; the explicit return
    // inside it must resolve to the block, not `m`.
    let rg = RubyPlugin.grammar("ruby").unwrap();
    assert_lambda_return_ignored(
        "ruby",
        &rg,
        "def m\n  xs.each do |x|\n    return inner()\n  end\nend\n",
        "m",
    );
}

/// Tail-of-block implicit return: the body-final expression of a *block* body
/// with no `return` keyword, for grammars whose `CfgNodeKinds.block_tail_returns`
/// is set (Rust / Scala / Ruby). The structural last-named-child pass attributes
/// the tail expression to the owning function exactly like a `return` operand.
fn assert_tail_block_return_binds(
    prefix: &'static str,
    grammar: &tree_sitter::Language,
    src: &str,
    fn_name: &str,
    fn_line: u32,
    fn_kind: SymbolKind,
) {
    let cfg = FlowConfig {
        strategy_prefix: prefix,
        assignment_query: "",
        type_guard_query: "",
        discriminant_guard_query: "",
        type_args_query: "",
        literal_type_kinds: &[],
    };
    let build_off = src.find("build").unwrap() as u32;
    let build_line = src[..build_off as usize].matches('\n').count() as u32;
    let symbols = vec![mk_sym(fn_name, fn_kind, fn_line)];
    let mut refs = vec![mk_call_ref("build", build_line, build_off)];

    let meta = run_flow_queries(src, grammar, &cfg, &symbols, &mut refs);

    assert_eq!(
        meta.flow_return_lhs.get(&0),
        Some(&0),
        "{prefix}: tail-of-block `build()` should bind to {fn_name} (symbol 0)"
    );
}

#[test]
fn flow_return_rust_tail_of_block() {
    // fn make_user() -> T { build() } — bare trailing call, no `return`, no `;`.
    use crate::languages::rust_lang::RustLangPlugin;
    let g = RustLangPlugin.grammar("rust").unwrap();
    assert_tail_block_return_binds(
        "rust",
        &g,
        "fn make_user() -> T {\n    build()\n}\n",
        "make_user",
        0,
        SymbolKind::Function,
    );
}

#[test]
fn flow_return_rust_tail_after_statement() {
    // fn make_user() -> T { let x = 1; build() } — tail follows a `let`.
    use crate::languages::rust_lang::RustLangPlugin;
    let g = RustLangPlugin.grammar("rust").unwrap();
    assert_tail_block_return_binds(
        "rust",
        &g,
        "fn make_user() -> T {\n    let x = 1;\n    build()\n}\n",
        "make_user",
        0,
        SymbolKind::Function,
    );
}

#[test]
fn flow_return_scala_tail_of_block() {
    // def makeUser: T = { build() } — Scala block body, last expr is the return.
    use crate::languages::scala::ScalaPlugin;
    let g = ScalaPlugin.grammar("scala").unwrap();
    assert_tail_block_return_binds(
        "scala",
        &g,
        "object O {\n  def makeUser: T = {\n    val x = 1\n    build()\n  }\n}\n",
        "makeUser",
        1,
        SymbolKind::Function,
    );
}

#[test]
fn flow_return_ruby_tail_of_block() {
    // def make_user\n build()\nend — Ruby implicit last-expression return.
    use crate::languages::ruby::RubyPlugin;
    let g = RubyPlugin.grammar("ruby").unwrap();
    assert_tail_block_return_binds(
        "ruby",
        &g,
        "def make_user\n  x = 1\n  build()\nend\n",
        "make_user",
        0,
        SymbolKind::Method,
    );
}

#[test]
fn flow_return_rust_trailing_semicolon_not_a_return() {
    // fn make_user() -> T { build(); } — the `;` makes `build()` an
    // expression_statement returning unit, NOT the function's return value.
    // The tail-of-block pass must reject a statement last child.
    use crate::languages::rust_lang::RustLangPlugin;
    let g = RustLangPlugin.grammar("rust").unwrap();
    let cfg = FlowConfig {
        strategy_prefix: "rust",
        assignment_query: "",
        type_guard_query: "",
        discriminant_guard_query: "",
        type_args_query: "",
        literal_type_kinds: &[],
    };
    let src = "fn make_user() -> T {\n    build();\n}\n";
    let build_off = src.find("build").unwrap() as u32;
    let symbols = vec![mk_sym("make_user", SymbolKind::Function, 0)];
    let mut refs = vec![mk_call_ref("build", 1, build_off)];
    let meta = run_flow_queries(src, &g, &cfg, &symbols, &mut refs);
    assert!(
        meta.flow_return_lhs.is_empty(),
        "a semicolon-terminated trailing expression returns unit, not the call type"
    );
}

#[test]
fn flow_return_rust_trailing_let_not_a_return() {
    // fn make_user() -> T { let r = build(); } — the block ends in a binding,
    // which returns unit; the tail-of-block pass must reject a `let_declaration`
    // last child even though its RHS carries the `build` ref.
    use crate::languages::rust_lang::RustLangPlugin;
    let g = RustLangPlugin.grammar("rust").unwrap();
    let cfg = FlowConfig {
        strategy_prefix: "rust",
        assignment_query: "",
        type_guard_query: "",
        discriminant_guard_query: "",
        type_args_query: "",
        literal_type_kinds: &[],
    };
    let src = "fn make_user() -> T {\n    let r = build();\n}\n";
    let build_off = src.find("build").unwrap() as u32;
    let symbols = vec![mk_sym("make_user", SymbolKind::Function, 0)];
    let mut refs = vec![mk_call_ref("build", 1, build_off)];
    let meta = run_flow_queries(src, &g, &cfg, &symbols, &mut refs);
    assert!(
        meta.flow_return_lhs.is_empty(),
        "a block ending in a `let` binding returns unit, not the bound call's type"
    );
}

#[test]
fn flow_return_ts_block_tail_not_a_return() {
    // function f() { build(); } — TS does NOT set `block_tail_returns`; a bare
    // trailing expression is a statement, not a return. The tail-of-block pass
    // must not fire for a `false`-flagged language (only `@return.expr` does).
    let cfg = FlowConfig {
        strategy_prefix: "ts",
        assignment_query: "",
        type_guard_query: "",
        discriminant_guard_query: "",
        type_args_query: "",
        literal_type_kinds: &[],
    };
    let src = "function f() {\n  build();\n}\n";
    let build_off = src.find("build").unwrap() as u32;
    let symbols = vec![mk_sym("f", SymbolKind::Function, 0)];
    let mut refs = vec![mk_call_ref("build", 1, build_off)];
    let meta = run_flow_queries(src, &ts_grammar(), &cfg, &symbols, &mut refs);
    assert!(
        meta.flow_return_lhs.is_empty(),
        "TS is not block_tail_returns — a trailing statement must not bind as a return"
    );
}

#[test]
fn kotlin_is_smartcast_narrows_in_if_block() {
    use crate::languages::kotlin::KotlinPlugin;

    // `if (x is Admin) { x.ban() }` smart-casts `x` to Admin in the block.
    let source = "fun f(x: Any) {\n  if (x is Admin) {\n    x.ban()\n  }\n}\n";
    let grammar = KotlinPlugin
        .grammar("kotlin")
        .expect("kotlin grammar must load");
    let cfg = KotlinPlugin.flow_config().expect("kotlin flow config");
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &grammar, cfg, &symbols, &mut refs);

    let n = meta
        .narrowings
        .iter()
        .find(|n| n.name == "x" && n.narrowed_type == "Admin")
        .unwrap_or_else(|| {
            panic!(
                "`is` smart-cast should narrow `x` to Admin; got {:?}",
                meta.narrowings
            )
        });
    assert!(n.byte_end > n.byte_start);
}

#[test]
fn kotlin_smartcast_dropped_on_reassignment() {
    use crate::languages::kotlin::KotlinPlugin;

    // A reassignment to `x` inside the smart-cast block invalidates the
    // narrowing from that point on — the use after `x = reset()` must NOT be
    // covered by the narrowing range.
    let source =
        "fun f(x: Any) {\n  if (x is Admin) {\n    x.ban()\n    x = reset()\n    x.bar()\n  }\n}\n";
    let grammar = KotlinPlugin
        .grammar("kotlin")
        .expect("kotlin grammar must load");
    let cfg = KotlinPlugin.flow_config().expect("kotlin flow config");
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let meta = run_flow_queries(source, &grammar, cfg, &symbols, &mut refs);

    let n = meta
        .narrowings
        .iter()
        .find(|n| n.name == "x")
        .expect("`is` smart-cast should narrow `x`");
    assert_eq!(n.narrowed_type, "Admin");

    let bar_pos = source.find("x.bar()").unwrap() as u32;
    assert!(
        n.byte_end <= bar_pos,
        "narrowing range [{}, {}) must not cover `x.bar()` at {bar_pos} (killed by reassignment)",
        n.byte_start,
        n.byte_end
    );
    // The use before the reassignment (`x.ban()`) stays narrowed.
    let ban_pos = source.find("x.ban()").unwrap() as u32;
    assert!(
        n.byte_start <= ban_pos && ban_pos < n.byte_end,
        "narrowing range [{}, {}) should still cover `x.ban()` at {ban_pos}",
        n.byte_start,
        n.byte_end
    );
}

/// Single-segment chain call ref, the shape the TS extractor emits for both a
/// bare call (`render(...)`) and a JSX component tag — a `chain` that wins the
/// assignment ref selection's chain-preference over chain-less candidates.
fn mk_chain_call_ref(target: &str, line: u32, byte_offset: u32) -> ExtractedRef {
    use crate::types::{ChainSegment, MemberChain, SegmentKind};
    ExtractedRef {
        chain: Some(MemberChain {
            segments: vec![ChainSegment {
                name: target.to_string(),
                node_kind: "identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
                byte_offset,
                declared_type_id: None,
                type_arg_ids: Vec::new(),
                is_call: false,
                call_args: Vec::new(),
            }],
        }),
        ..mk_call_ref(target, line, byte_offset)
    }
}

#[test]
fn flow_assignment_skips_refs_inside_callback_argument() {
    // `const rendered = render(() => f());` — the RHS is the `render(...)` call;
    // the `f()` ref lives inside the arrow ARGUMENT, so it belongs to the
    // callback body, not to the value bound to `rendered`. Both refs carry a
    // chain (the shape the TS extractor emits for calls and JSX tags), and `f`
    // sits further right than `render`, so the rightmost-chain selection would
    // wrongly bind `rendered` to `f`'s return without the nested-callback skip.
    let source = "const rendered = render(() => f());\n";
    let render_off = source.find("render(").unwrap() as u32;
    let f_off = source.find("f()").unwrap() as u32;
    let symbols = vec![mk_sym("rendered", SymbolKind::Variable, 0)];
    let mut refs = vec![
        mk_chain_call_ref("render", 0, render_off),
        mk_chain_call_ref("f", 0, f_off),
    ];

    let meta = run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs);

    assert_eq!(
        meta.flow_binding_lhs.get(&0),
        Some(&0),
        "`rendered` must bind to ref 0 (the outer `render` call), not the callback-nested `f`"
    );
    assert!(
        !meta.flow_binding_lhs.contains_key(&1),
        "the `f` ref inside the arrow argument must not be the binding initializer"
    );
}

// ---------------------------------------------------------------------------
// Declared-type seeding — annotation capture + literal-kind table
// ---------------------------------------------------------------------------

/// Run the TS assignment query over a single-declaration source line and
/// return the populated `FlowMeta`. The declared variable name is extracted
/// from the source by scanning for the first identifier that follows
/// `const ` or `let ` or `var `.
fn run_ts_assignment(source: &str) -> crate::types::FlowMeta {
    // Extract the variable name: first word after let/const/var.
    let var_name = source
        .split_whitespace()
        .skip(1)
        .next()
        .unwrap_or("x")
        .trim_end_matches(':');
    let symbols = vec![mk_sym(var_name, SymbolKind::Variable, 0)];
    let mut refs: Vec<ExtractedRef> = Vec::new();
    run_flow_queries(source, &ts_grammar(), &TS_TEST_FLOW, &symbols, &mut refs)
}

#[test]
fn ts_array_annotation_seeds_decl_type() {
    // `const queries: Array<unknown> = []` — the @type capture lands the full
    // annotation text in flow_binding_decl_type; the type interner (not this
    // capture) decomposes `Array<unknown>` into the head plus its element arg.
    // The RHS `[]` has no resolvable ref, so without the annotation capture
    // this binding would produce no type at all.
    let meta = run_ts_assignment("const queries: Array<unknown> = []");
    assert_eq!(
        meta.flow_binding_decl_type.get(&0).map(String::as_str),
        Some("Array<unknown>"),
        "annotated Array<unknown> must seed flow_binding_decl_type with the full text"
    );
}

#[test]
fn ts_array_literal_no_annotation_seeds_decl_type() {
    // `let list = ['a','b']` — no annotation, no resolvable ref. The
    // literal-kind table maps the `array` node kind to `Array` so the chain
    // walker can type `list.push(...)` without an explicit annotation.
    let meta = run_ts_assignment("let list = ['a', 'b']");
    assert_eq!(
        meta.flow_binding_decl_type.get(&0).map(String::as_str),
        Some("Array"),
        "bare array literal must seed flow_binding_decl_type with \"Array\""
    );
}
