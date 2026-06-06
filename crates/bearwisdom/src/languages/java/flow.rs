// =============================================================================
// java/flow.rs — R5 Sprint 4 Java FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

/// Return-expression query for body-based return-type inference (INFER-3).
/// Captures the returned expression of every `return <expr>`;
/// `flow::run_return_query` resolves the owning method by ancestor-walk
/// (dropping a return whose nearest function is a nested `lambda_expression`).
pub const JAVA_RETURN_QUERY: &str = r#"
    (return_statement (_) @return.expr)
"#;

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

    // `if (x instanceof Foo) { ... }` narrows `x`; `if (x instanceof Foo f)`
    // (Java 16+ pattern binding) additionally types the binding `f` as Foo —
    // the pattern variable follows the type_identifier positionally.
    type_guard_query: r#"
        (if_statement
            condition: (parenthesized_expression
                (instanceof_expression
                    (identifier) @guard.local
                    (type_identifier) @guard.type))
            consequence: (block) @guard.body)

        (if_statement
            condition: (parenthesized_expression
                (instanceof_expression
                    (type_identifier) @guard.type
                    (identifier) @guard.local))
            consequence: (block) @guard.body)
    "#,

    // `obj.<T>method()` / `Collections.<T>emptyList()` — Java's call-site
    // type arguments come before the method name.
    discriminant_guard_query: "",
    type_args_query: r#"
        (method_invocation
            type_arguments: (type_arguments
                (type_identifier) @call.type_arg)
            name: (identifier) @call.method)
    "#,
};
