// =============================================================================
// dart/flow.rs — Dart FlowConfig (CFG-native `is T` flow promotion).
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static DART_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "dart",

    // `var x = <expr>` / `final x = <expr>` parse as
    // `initialized_variable_definition` with `name` + `value` fields;
    // reassignment is `assignment_expression` with `left`/`right`. Single-LHS
    // forms only.
    assignment_query: r#"
        (initialized_variable_definition
            name: (identifier) @lhs
            value: (_) @rhs)

        (assignment_expression
            left: (assignable_expression
                (identifier) @lhs)
            right: (_) @rhs)
    "#,

    // `if (x is Foo) { ... }` promotes `x` to `Foo` in the then-block. The
    // condition is the positional `type_test_expression` child of the
    // if_statement; the then-block is the `consequence` field.
    type_guard_query: r#"
        (if_statement
            (type_test_expression
                (identifier) @guard.local
                (type_test (type_identifier) @guard.type))
            consequence: (block) @guard.body)
    "#,

    discriminant_guard_query: "",
    type_args_query: "",
};
