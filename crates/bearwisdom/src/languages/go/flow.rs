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
    // We match the simple single-LHS form; multi-value returns are left
    // to future work.
    assignment_query: r#"
        (short_var_declaration
            left: (expression_list
                (identifier) @lhs)
            right: (expression_list
                (_) @rhs))

        (assignment_statement
            left: (expression_list
                (identifier) @lhs)
            right: (expression_list
                (_) @rhs))

        (var_spec
            name: (identifier) @lhs
            value: (expression_list
                (_) @rhs))
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

    // Go's generic type-argument node structure varies between grammar
    // releases; leave empty in v1 to avoid compilation failures. The chain
    // walker already honors seg.type_args if extractors populate them.
    discriminant_guard_query: "",
    type_args_query: "",
};
