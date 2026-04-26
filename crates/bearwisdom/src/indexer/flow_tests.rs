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
            value: (_) @rhs)

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)
    "#,
    type_guard_query: r#"
        (if_statement
            condition: (parenthesized_expression
                (binary_expression
                    left: (identifier) @guard.local
                    right: (identifier) @guard.type))
            consequence: (statement_block) @guard.body)
    "#,
    type_args_query: r#"
        (call_expression
            function: (member_expression
                property: (property_identifier) @call.method)
            type_arguments: (type_arguments
                (type_identifier) @call.type_arg))
    "#,
};

fn ts_grammar() -> tree_sitter::Language {
    TypeScriptPlugin.grammar("typescript").expect("TS grammar must load")
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
    }
}

fn mk_call_ref(target: &str, line: u32, byte_offset: u32) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line,
        module: None,
        chain: None,
        byte_offset,
        namespace_segments: Vec::new(),
    }
}

#[test]
fn flow_assignment_binds_lhs_to_rhs_ref() {
    let source = "const x = foo();\n";
    // "const x = foo();" byte positions:
    //   'const ' = 0..6, 'x' = 6, ' = ' = 7..10, 'foo' = 10..13, '()' = 13..15
    let symbols = vec![
        mk_sym("x", SymbolKind::Variable, 0),
    ];
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
fn flow_type_args_populate_chain_segment() {
    use crate::types::{ChainSegment, MemberChain, SegmentKind};

    let source = "repo.findOne<User>();\n";
    // Byte positions:
    //   'repo' = 0..4
    //   '.findOne' = 4..12
    //   findOne at bytes 5..12 (property_identifier: 'findOne')
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs = vec![ExtractedRef {
        source_symbol_index: 0,
        target_name: "findOne".to_string(),
        kind: EdgeKind::Calls,
        line: 0,
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
                },
                ChainSegment {
                    name: "findOne".to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: Vec::new(),
                    optional_chaining: false,
                },
            ],
        }),
        byte_offset: 5, // inside the `findOne` span (5..12)
            namespace_segments: Vec::new(),
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
