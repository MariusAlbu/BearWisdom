// =============================================================================
// swift/flow.rs — Swift FlowConfig (CFG-native `is T` / `as? T` / `if let`
// flow promotion).
// =============================================================================

use crate::indexer::flow::FlowConfig;

fn if_consequence_body(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut cursor = node.walk();
    let body = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == "statements");
    body
}

pub static SWIFT_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "swift",

    // `let x = <expr>` / `var x = <expr>` parse as `property_declaration` with
    // a `value` field; the bound name is a `pattern` child. Reassignment is
    // `assignment` with `target`/`result`. Single-LHS forms only.
    assignment_query: r#"
        (property_declaration
            name: (pattern (simple_identifier) @lhs)
            value: (_) @rhs)

        (assignment
            target: (directly_assignable_expression
                (simple_identifier) @lhs)
            result: (_) @rhs)
    "#,

    // Two Swift narrowing forms, both attaching the guard to the then-block
    // (`statements`) by byte range — the if_statement lists its branches
    // positionally with no consequence field:
    //   if x is T { ... }            — `check_expression` narrows `x` to T
    //   if let y = x as? T { ... }   — optional downcast binds `y` as T
    type_guard_query: r#"
        (if_statement
            (check_expression
                (simple_identifier) @guard.local
                (user_type (type_identifier) @guard.type))
            (statements) @guard.body)

        (if_statement
            (simple_identifier) @guard.local
            (as_expression
                (user_type (type_identifier) @guard.type))
            (statements) @guard.body)
    "#,

    discriminant_guard_query: "",
    type_args_query: "",
    literal_type_kinds: &[],
};

pub const SWIFT_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
    crate::indexer::flow_cfg::CfgNodeKinds {
        function_kinds: &["function_declaration", "init_declaration", "lambda_literal"],
        block_kinds: &["statements"],
        if_kind: "if_statement",
        if_consequence_field: "",
        if_consequence_body: Some(if_consequence_body),
        if_alternative_field: "",
        if_alternative_body: None,
        if_condition_field: "condition",
        assignment_kind: "assignment",
        assignment_lhs_field: "target",
        declarator_kind: "property_declaration",
        declarator_name_field: "name",
        binding_name_kinds: &["simple_identifier"],
        definition_name_kinds: &["identifier"],
        bare_return_name_kinds: &["identifier"],
        function_name_fields: &["name"],
        loop_kinds: &["while_statement", "for_statement", "repeat_while_statement"],
        loop_body_field: "body",
        loop_condition_field: Some("condition"),
        switch_kinds: &["switch_statement"],
        switch_value_field: "expr",
        switch_body_field: None,
        switch_case_kinds: &["switch_entry"],
        switch_default_kinds: &[],
        // `function_body` wraps the `statements` block; descend through it.
        transparent_kinds: &["function_body"],
        implicit_return_candidate: None,
        condition_true_guard: None,
    };
