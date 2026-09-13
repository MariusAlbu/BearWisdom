// =============================================================================
// php/flow.rs — R5 Sprint 4 PHP FlowConfig
//
// PHP has no generics, so `type_args_query` is empty.
// =============================================================================

use crate::indexer::flow::FlowConfig;

/// Return-expression query for body-based return-type inference (INFER-3).
/// Captures the returned expression of every `return <expr>`;
/// `flow::run_return_query` resolves the owning function/method by
/// ancestor-walk (dropping a return whose nearest function is a nested
/// `anonymous_function` / `arrow_function`).
pub const PHP_RETURN_QUERY: &str = r#"
    (return_statement (_) @return.expr)
"#;

pub static PHP_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "php",

    // `$x = <expr>;` — PHP uses `assignment_expression` with `left` being a
    // `variable_name` and `right` being the value. No separate declaration
    // form; assignments double as declarations on first use.
    assignment_query: r#"
        (assignment_expression
            left: (variable_name
                (name) @lhs)
            right: (_) @rhs)

        (assignment_expression
            left: (member_access_expression
                name: (name) @lhs.member)
            right: (_) @rhs)

        (simple_parameter
            type: (_) @type
            name: (variable_name (name) @lhs.param))

        (simple_parameter
            name: (variable_name (name) @lhs.param))

        (property_promotion_parameter
            type: (_) @type
            name: (variable_name (name) @lhs.param))
    "#,

    // `if ($x instanceof Foo) { ... }` — PHP narrowing form.
    type_guard_query: r#"
        (if_statement
            condition: (parenthesized_expression
                (binary_expression
                    left: (variable_name
                        (name) @guard.local)
                    right: (name) @guard.type))
            body: (compound_statement) @guard.body)
    "#,

    // PHP has no call-site generic arguments.
    discriminant_guard_query: "",
    type_args_query: "",
    literal_type_kinds: &[],
};

pub const PHP_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
    crate::indexer::flow_cfg::CfgNodeKinds {
        function_kinds: &[
            "function_definition",
            "method_declaration",
            "anonymous_function",
            "arrow_function",
            "anonymous_function_creation_expression",
        ],
        block_kinds: &["compound_statement"],
        if_kind: "if_statement",
        if_consequence_field: "body",
        if_consequence_body: None,
        if_alternative_field: "alternative",
        if_alternative_body: None,
        if_condition_field: "condition",
        assignment_kind: "assignment_expression",
        assignment_lhs_field: "left",
        declarator_kind: "__php_no_declarator__",
        declarator_name_field: "name",
        binding_name_kinds: &["name", "variable_name"],
        definition_name_kinds: &["identifier"],
        bare_return_name_kinds: &["identifier"],
        function_name_fields: &["name"],
        loop_kinds: &[
            "while_statement",
            "for_statement",
            "foreach_statement",
            "do_statement",
        ],
        loop_body_field: "body",
        loop_condition_field: Some("condition"),
        switch_kinds: &["switch_statement"],
        switch_value_field: "condition",
        switch_body_field: Some("body"),
        switch_case_kinds: &["case_statement"],
        switch_default_kinds: &["default_statement"],
        transparent_kinds: &[],
        implicit_return_candidate: None,
        condition_true_guard: None,
    };
