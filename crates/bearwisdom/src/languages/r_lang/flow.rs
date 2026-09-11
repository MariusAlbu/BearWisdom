// =============================================================================
// r_lang/flow.rs — R FlowConfig for local-variable type inference
//
// R uses `binary_operator` for all binary expressions. Assignment operators
// are distinguished by the `operator` field text: `<-`, `=`, `<<-`.
// Right-arrow (`->`) is skipped — it reverses operand order, so `@lhs`
// would capture the value expression and `@rhs` the variable name, which
// is the opposite of what the flow runner expects.
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static R_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "r",

    // `x <- <expr>` and `x = <expr>` — the two dominant R assignment forms.
    // `binary_operator` has named fields `lhs`, `operator`, and `rhs`.
    // The `#eq?` predicates guard against arithmetic and logical operators
    // that share the same node kind.
    assignment_query: r#"
        (binary_operator
            lhs: (identifier) @lhs
            operator: _ @op
            rhs: (_) @rhs
            (#match? @op "^(<-|=|<<-)$"))
    "#,

    // R has no standard structural type-narrowing construct analogous to
    // Python's `isinstance` or TypeScript's `instanceof` that produces a
    // scoped narrowing block.
    type_guard_query: "",

    // R has no call-site generic type arguments.
    discriminant_guard_query: "",
    type_args_query: "",
    literal_type_kinds: &[],
};

pub const R_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
    crate::indexer::flow_cfg::CfgNodeKinds {
        function_kinds: &["function_definition"],
        block_kinds: &["braced_expression"],
        if_kind: "if_statement",
        if_consequence_field: "consequence",
        if_consequence_body: None,
        if_alternative_field: "alternative",
        if_alternative_body: None,
        if_condition_field: "condition",
        assignment_kind: "__r_no_assignment__",
        assignment_lhs_field: "left",
        declarator_kind: "__r_no_declarator__",
        declarator_name_field: "name",
        binding_name_kinds: &["identifier"],
        definition_name_kinds: &["identifier"],
        bare_return_name_kinds: &["identifier"],
        function_name_fields: &["name"],
        loop_kinds: &["while_statement", "for_statement", "repeat_statement"],
        loop_body_field: "body",
        loop_condition_field: Some("condition"),
        switch_kinds: &[],
        switch_value_field: "value",
        switch_body_field: None,
        switch_case_kinds: &[],
        switch_default_kinds: &[],
        transparent_kinds: &[],
        // R is expression-oriented (a function's last expression is its value), but
        // R has no return query wired, so the tail pass has nothing to attribute.
        implicit_return_candidate: None,
        condition_true_guard: None,
    };
