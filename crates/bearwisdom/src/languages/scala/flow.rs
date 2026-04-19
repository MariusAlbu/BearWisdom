// =============================================================================
// scala/flow.rs — R5 Sprint 4 Scala FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static SCALA_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "scala",

    // `val/var x = <expr>` — `val_definition`/`var_definition` in
    // tree-sitter-scala. Captures the name pattern and value expression.
    assignment_query: r#"
        (val_definition
            pattern: (identifier) @lhs
            value: (_) @rhs)

        (var_definition
            pattern: (identifier) @lhs
            value: (_) @rhs)

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)
    "#,

    // Pattern-match narrowing via `case Foo(_) =>` is too general to query
    // cleanly. v1 leaves this empty; Scala's strong inference already
    // surfaces types via declared_type on pattern bindings.
    type_guard_query: "",

    // `repo.findOne[User]()` — Scala type arguments on calls.
    type_args_query: r#"
        (generic_function
            function: (field_expression
                field: (identifier) @call.method)
            type_arguments: (type_arguments
                (type_identifier) @call.type_arg))
    "#,
};
