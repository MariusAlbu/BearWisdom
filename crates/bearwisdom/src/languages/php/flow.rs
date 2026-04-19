// =============================================================================
// php/flow.rs — R5 Sprint 4 PHP FlowConfig
//
// PHP has no generics, so `type_args_query` is empty.
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static PHP_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "php",

    // `$x = <expr>;` — PHP uses `assignment_expression` with `left` being a
    // `variable_name` and `right` being the value. No separate declaration
    // form; assignments double as declarations on first use.
    assignment_query: r#"
        (assignment_expression
            left: (variable_name
                (name) @lhs)
            right: (_) @rhs)
    "#,

    // `if ($x instanceof Foo) { ... }` — PHP narrowing form.
    type_guard_query: r#"
        (if_statement
            condition: (parenthesized_expression
                (binary_expression
                    left: (variable_name
                        (name) @guard.local)
                    right: (name) @guard.type))
            body: (compound_statement) @guard.body)
    "#,

    // PHP has no call-site generic arguments.
    type_args_query: "",
};
