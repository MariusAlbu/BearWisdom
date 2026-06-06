// =============================================================================
// kotlin/flow.rs — R5 Sprint 4 Kotlin FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

/// Return-expression query for body-based return-type inference (INFER-3).
/// Captures the returned expression of every `return e` (a `return_expression`),
/// plus the concise expression body of `fun f() = e` via `@return.tail` (the
/// child of `function_body`). `flow::run_return_query` resolves the owning
/// function by ancestor-walk (dropping a return whose nearest function is a
/// nested lambda), and skips a `@return.tail` whose kind is a `block` — a
/// block-bodied `fun f() { … }` returns from its explicit `return` (the
/// `@return.expr` arm), not from the block node.
pub const KOTLIN_RETURN_QUERY: &str = r#"
    (return_expression (_) @return.expr)
    (function_body (_) @return.tail)
"#;

pub static KOTLIN_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "kotlin",

    // Kotlin grammar node names differ across releases (tree-sitter-kotlin-ng);
    // v1 ships a minimal assignment query. Narrowing + type args stay empty
    // pending grammar verification — the resolver degrades to pre-R5 behavior
    // for those features.
    assignment_query: r#"
        (property_declaration
            (variable_declaration
                (identifier) @lhs)
            (_) @rhs)
    "#,

    type_guard_query: "",
    discriminant_guard_query: "",
    type_args_query: "",
};
