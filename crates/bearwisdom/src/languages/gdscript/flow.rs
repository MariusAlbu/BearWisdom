// =============================================================================
// gdscript/flow.rs — GDScript FlowConfig (CFG-native `is T` flow promotion).
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static GDSCRIPT_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "gdscript",

    // `var x = <expr>` / `const x = <expr>` parse as `variable_statement` /
    // `const_statement` with `name` + `value` fields; reassignment is
    // `assignment` with `left`/`right`. Single-LHS forms only.
    assignment_query: r#"
        (variable_statement
            name: (name) @lhs
            value: (_) @rhs)

        (const_statement
            name: (name) @lhs
            value: (_) @rhs)

        (assignment
            left: (identifier) @lhs
            right: (_) @rhs)
    "#,

    // `if x is Foo:` promotes `x` to `Foo` in the then-block. The condition is
    // a `binary_operator` with the anonymous `is` token; `x` is the left
    // identifier, `Foo` the right identifier. The then-block is the `body`
    // field.
    type_guard_query: r#"
        (if_statement
            condition: (binary_operator
                (identifier) @guard.local
                "is"
                (identifier) @guard.type)
            body: (body) @guard.body)
    "#,

    discriminant_guard_query: "",
    type_args_query: "",
    literal_type_kinds: &[],
};

pub const GDSCRIPT_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
    crate::indexer::flow_cfg::CfgNodeKinds {
        function_kinds: &["function_definition", "lambda"],
        block_kinds: &["body"],
        if_kind: "if_statement",
        if_consequence_field: "body",
        if_consequence_body: None,
        if_alternative_field: "alternative",
        if_alternative_body: None,
        if_condition_field: "condition",
        assignment_kind: "assignment",
        assignment_lhs_field: "left",
        declarator_kind: "variable_statement",
        declarator_name_field: "name",
        binding_name_kinds: &["identifier"],
        definition_name_kinds: &["identifier"],
        bare_return_name_kinds: &["identifier"],
        function_name_fields: &["name"],
        loop_kinds: &["while_statement", "for_statement"],
        loop_body_field: "body",
        loop_condition_field: Some("condition"),
        switch_kinds: &["match_statement"],
        switch_value_field: "value",
        switch_body_field: Some("body"),
        switch_case_kinds: &["pattern_section"],
        switch_default_kinds: &[],
        transparent_kinds: &[],
        implicit_return_candidate: None,
        condition_true_guard: None,
    };
