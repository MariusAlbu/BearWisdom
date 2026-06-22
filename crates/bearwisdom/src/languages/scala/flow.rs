// =============================================================================
// scala/flow.rs — R5 Sprint 4 Scala FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

/// Return-expression query for body-based return-type inference (INFER-3).
/// Captures the returned expression of every explicit `return e`
/// (`return_expression`), plus the concise expression body of `def f = e` via
/// `@return.tail` (the `function_definition` body field). `flow::run_return_query`
/// resolves the owning def by ancestor-walk (dropping a return whose nearest
/// function is a nested lambda), and skips a `@return.tail` whose kind is a
/// `block` — `def f = { … }` returns from its block's final expression, which
/// the structural tail-of-block pass attributes (gated on
/// `CfgNodeKinds.block_tail_returns`), not the block node itself.
pub const SCALA_RETURN_QUERY: &str = r#"
    (return_expression (_) @return.expr)
    (function_definition body: (_) @return.tail)
"#;

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
    discriminant_guard_query: "",
    type_args_query: r#"
        (generic_function
            function: (field_expression
                field: (identifier) @call.method)
            type_arguments: (type_arguments
                (type_identifier) @call.type_arg))
    "#,
    literal_type_kinds: &[],
};
