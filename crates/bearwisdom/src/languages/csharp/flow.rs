// =============================================================================
// csharp/flow.rs — R5 Sprint 4 C# FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static CSHARP_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "csharp",

    // `var x = foo();` / `Foo x = foo();` / reassignment `x = foo();`.
    // tree-sitter-c-sharp uses `variable_declarator` with `name` and
    // `value` fields, and `assignment_expression` for reassignment.
    assignment_query: r#"
        (variable_declarator
            name: (identifier) @lhs
            (_) @rhs)

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)
    "#,

    // `if (x is Foo) { ... }` — C# pattern-matching narrowing.
    type_guard_query: r#"
        (if_statement
            condition: (is_pattern_expression
                (identifier) @guard.local
                (constant_pattern
                    (identifier) @guard.type))
            consequence: (block) @guard.body)
    "#,

    // Generic method invocation: `repo.FindOne<User>()` — `generic_name`
    // holds the identifier and type_argument_list.
    type_args_query: r#"
        (invocation_expression
            function: (member_access_expression
                name: (generic_name
                    (identifier) @call.method
                    (type_argument_list
                        (identifier) @call.type_arg))))
    "#,
};
