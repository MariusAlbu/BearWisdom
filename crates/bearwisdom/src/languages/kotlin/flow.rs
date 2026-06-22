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

    // `val/var x = <expr>` is a `property_declaration` (forward inference); a
    // later `x = <expr>` is an `assignment` whose `left` field is the bare
    // identifier — captured so `kill_narrowings_at_reassignments` can truncate
    // a smart-cast that straddles a re-def of the same name.
    assignment_query: r#"
        (property_declaration
            (variable_declaration
                (identifier) @lhs)
            (_) @rhs)

        (assignment
            left: (identifier) @lhs)
    "#,

    // `if (x is T) { … }` smart-casts `x` to `T` in the braced consequence.
    // Grammar (tree-sitter-kotlin-ng): the if-condition is the `is_expression`
    // directly (no parenthesized wrapper); `left` is the narrowed local, `right`
    // is the `user_type` whose inner `identifier` is the narrowed type; the
    // consequence is the positional `block` child (the if has no field for it).
    // Braceless bodies (`if (x is T) x.foo()`) use `control_structure_body`
    // instead of `block` and are intentionally not matched (decline, no bind).
    type_guard_query: r#"
        (if_expression
            condition: (is_expression
                left: (identifier) @guard.local
                right: (user_type (identifier) @guard.type))
            (block) @guard.body)
    "#,

    discriminant_guard_query: "",
    type_args_query: "",
    literal_type_kinds: &[],
};
