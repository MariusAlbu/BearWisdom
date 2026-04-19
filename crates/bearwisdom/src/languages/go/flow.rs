// =============================================================================
// go/flow.rs — R5 Sprint 4 Go FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

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

    // Go's narrowing via type assertion: `if v, ok := x.(Foo); ok { ... }`.
    // Captures `v` (the narrowed name), `Foo`, and the if body.
    type_guard_query: r#"
        (if_statement
            initializer: (short_var_declaration
                left: (expression_list
                    (identifier) @guard.local)
                right: (expression_list
                    (type_assertion_expression
                        type: (type_identifier) @guard.type)))
            consequence: (block) @guard.body)
    "#,

    // Go's generic type-argument node structure varies between grammar
    // releases; leave empty in v1 to avoid compilation failures. The chain
    // walker already honors seg.type_args if extractors populate them.
    type_args_query: "",
};
