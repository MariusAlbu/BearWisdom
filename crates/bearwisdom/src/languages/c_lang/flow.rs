// =============================================================================
// c_lang/flow.rs — R5 Sprint 4 C/C++ FlowConfig
//
// C has no runtime type narrowing; C++ has `dynamic_cast` but it's awkward to
// query structurally. v1 ships assignment + C++ template type args; narrowing
// stays empty.
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static C_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "c",

    // `Type x = <expr>;` → `declaration` holds an `init_declarator`.
    // Reassignment: `assignment_expression`.
    assignment_query: r#"
        (init_declarator
            declarator: (identifier) @lhs
            value: (_) @rhs)

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)
    "#,

    // No reliable, cheap narrowing query for C/C++. Skip.
    type_guard_query: "",

    // C++ template args require tree-sitter-cpp (not tree-sitter-c); we use
    // tree-sitter-c for `.c`/`.h` files where templates don't apply. Leave
    // this empty — cross-dialect query support is future work.
    discriminant_guard_query: "",
    type_args_query: "",
    literal_type_kinds: &[],
};

pub const C_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
    crate::indexer::flow_cfg::CfgNodeKinds {
        function_kinds: &["function_definition"],
        block_kinds: &["compound_statement"],
        if_kind: "if_statement",
        if_consequence_field: "consequence",
        if_consequence_body: None,
        if_alternative_field: "alternative",
        if_alternative_body: None,
        if_condition_field: "condition",
        assignment_kind: "assignment_expression",
        assignment_lhs_field: "left",
        declarator_kind: "init_declarator",
        declarator_name_field: "declarator",
        binding_name_kinds: &["identifier"],
        definition_name_kinds: &["identifier"],
        bare_return_name_kinds: &["identifier"],
        function_name_fields: &["declarator", "name"],
        loop_kinds: &["while_statement", "for_statement", "do_statement"],
        loop_body_field: "body",
        loop_condition_field: Some("condition"),
        switch_kinds: &["switch_statement"],
        switch_value_field: "condition",
        // C's switch puts case_statement children directly in the compound_statement
        // body wrapper (no separate switch_body field shape).
        switch_body_field: Some("body"),
        switch_case_kinds: &["case_statement"],
        switch_default_kinds: &[],
        transparent_kinds: &[],
        implicit_return_candidate: None,
        condition_true_guard: None,
    };
