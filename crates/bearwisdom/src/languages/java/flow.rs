// =============================================================================
// java/flow.rs — R5 Sprint 4 Java FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static JAVA_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "java",

    // `Foo x = <expr>;` — `local_variable_declaration` has a `declarator`
    // (`variable_declarator`) with `name` and `value` fields. Java 10+
    // `var x = ...` uses the same node with `var` as the type.
    // Also covers reassignment via `assignment_expression`.
    assignment_query: r#"
        (local_variable_declaration
            declarator: (variable_declarator
                name: (identifier) @lhs
                value: (_) @rhs))

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)
    "#,

    // `if (x instanceof Foo) { ... }` — Java's narrowing form.
    type_guard_query: r#"
        (if_statement
            condition: (parenthesized_expression
                (instanceof_expression
                    (identifier) @guard.local
                    (type_identifier) @guard.type))
            consequence: (block) @guard.body)
    "#,

    // `obj.<T>method()` / `Collections.<T>emptyList()` — Java's call-site
    // type arguments come before the method name.
    type_args_query: r#"
        (method_invocation
            type_arguments: (type_arguments
                (type_identifier) @call.type_arg)
            name: (identifier) @call.method)
    "#,
};
