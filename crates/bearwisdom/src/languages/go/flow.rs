// =============================================================================
// go/flow.rs — R5 Sprint 4 Go FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

/// Return-expression query for body-based return-type inference (INFER-3).
/// Captures the returned value of every `return`; `flow::run_return_query`
/// resolves the owning function by ancestor-walk (dropping a return whose
/// nearest function is a nested `func_literal`). Go wraps return values in an
/// `expression_list`; the single-value form is captured (multi-value returns
/// are a deeper sub-case).
pub const GO_RETURN_QUERY: &str = r#"
    (return_statement (expression_list (_) @return.expr))
"#;

pub static GO_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "go",

    // `x := <expr>` → `short_var_declaration`. `var x = <expr>` →
    // `var_declaration` + `var_spec`. Reassignment: `assignment_statement`.
    // Single-LHS form only (multi-value `a, b := …` is a deeper sub-case).
    //
    // The RHS is the EXPLICIT alternation of the chain-bearing node kinds
    // (call / selector / identifier / type-assertion / composite-literal)
    // rather than an unrestricted `(_)` wildcard. Tree-sitter compiles a `(_)`
    // inside an `(expression_list …)` into a query automaton that expands a
    // state per possible `_expression` production at that point; on certain Go
    // source shapes that combinatorial expansion allocated ~400MB and OOM'd.
    // Enumerating the node kinds the resolver actually consumes collapses the
    // automaton to a fixed, small state set.
    assignment_query: r#"
        (short_var_declaration
            left: (expression_list
                (identifier) @lhs)
            right: (expression_list
                [(call_expression)
                 (selector_expression)
                 (identifier)
                 (type_assertion_expression)
                 (composite_literal)] @rhs))

        (assignment_statement
            left: (expression_list
                (identifier) @lhs)
            right: (expression_list
                [(call_expression)
                 (selector_expression)
                 (identifier)
                 (type_assertion_expression)
                 (composite_literal)] @rhs))

        (var_spec
            name: (identifier) @lhs
            value: (expression_list
                [(call_expression)
                 (selector_expression)
                 (identifier)
                 (type_assertion_expression)
                 (composite_literal)] @rhs))
    "#,

    // Two Go narrowing forms:
    //   if v, ok := x.(Foo); ok { ... }     — type assertion, narrows `v` to Foo
    //   switch v := x.(type) { case *Foo: }  — type switch, narrows `v` per case
    // For the type switch, the alias `v` narrows to each case's type within that
    // case body (both the bare and pointer forms).
    type_guard_query: r#"
        (if_statement
            initializer: (short_var_declaration
                left: (expression_list
                    (identifier) @guard.local)
                right: (expression_list
                    (type_assertion_expression
                        type: (type_identifier) @guard.type)))
            consequence: (block) @guard.body)

        (type_switch_statement
            alias: (expression_list
                (identifier) @guard.local)
            (type_case
                [(type_identifier) @guard.type
                 (pointer_type (type_identifier) @guard.type)]) @guard.body)
    "#,

    // Expression-switch discriminant: `switch x.field { case "lit": ... }`
    // narrows `x` per case to the branch carrying that discriminant literal.
    // @guard.local is the scrutinee receiver, @guard.prop the selected field,
    // @guard.literal the case's string literal (one per case value),
    // @guard.body the case node whose range scopes the narrowing.
    discriminant_guard_query: r#"
        (expression_switch_statement
            value: (selector_expression
                operand: (identifier) @guard.local
                field: (field_identifier) @guard.prop)
            (expression_case
                value: (expression_list
                    [(interpreted_string_literal) @guard.literal
                     (raw_string_literal) @guard.literal])) @guard.body)
    "#,

    // Go's generic type-argument node structure varies between grammar
    // releases; leave empty in v1 to avoid compilation failures. The chain
    // walker already honors seg.type_args if extractors populate them.
    type_args_query: "",
};
