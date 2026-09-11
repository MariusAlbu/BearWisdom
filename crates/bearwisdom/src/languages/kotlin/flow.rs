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

        (parameter
            (identifier) @lhs.param
            (_) @type)

        (lambda_parameters
            (variable_declaration
                (identifier) @lhs.param))

        (property_declaration
            (variable_declaration
                (identifier) @lhs
                (_) @type))
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

pub const KOTLIN_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
    crate::indexer::flow_cfg::CfgNodeKinds {
        function_kinds: &["function_declaration", "lambda_literal", "function_literal"],
        block_kinds: &["block"],
        if_kind: "if_expression",
        if_consequence_field: "consequence",
        if_consequence_body: None,
        if_alternative_field: "alternative",
        if_alternative_body: None,
        if_condition_field: "condition",
        assignment_kind: "assignment",
        assignment_lhs_field: "left",
        declarator_kind: "property_declaration",
        declarator_name_field: "name",
        binding_name_kinds: &["simple_identifier", "identifier"],
        definition_name_kinds: &["identifier"],
        bare_return_name_kinds: &["identifier"],
        function_name_fields: &["name"],
        loop_kinds: &["while_statement", "for_statement", "do_while_statement"],
        loop_body_field: "body",
        loop_condition_field: Some("condition"),
        switch_kinds: &["when_expression"],
        switch_value_field: "value",
        switch_body_field: None,
        switch_case_kinds: &["when_entry"],
        switch_default_kinds: &[],
        transparent_kinds: &["function_body"],
        // A Kotlin block body returns only via an explicit `return`; a bare
        // trailing expression is a statement. The concise `= expr` body (implicit
        // return) is already covered by the `@return.tail` query arm.
        implicit_return_candidate: None,
        condition_true_guard: None,
    };
