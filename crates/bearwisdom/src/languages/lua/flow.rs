// =============================================================================
// lua/flow.rs — Lua FlowConfig for local-var type binding
//
// Lua has two assignment forms that this config covers:
//
//   1. Global/bare:  `x = expr`
//      Tree-sitter:  assignment_statement
//                      variable_list → identifier @lhs
//                      expression_list → _ @rhs
//
//   2. Local:        `local x = expr`
//      Tree-sitter:  variable_declaration
//                      assignment_statement   ← inner node
//                        variable_list → identifier @lhs
//                        expression_list → _ @rhs
//
// The `assignment_query` matches the inner `assignment_statement` in both
// cases. The outer `variable_declaration` wrapper is transparent — the inner
// node is a direct child of `variable_declaration` and the query cursor
// descends into it naturally.
//
// Multi-assignment (`a, b = f()`) binds only the first name — the RHS
// byte range correlates to whichever ref falls inside it, which is correct
// for the common single-name form. Table field LHS (`t.k = expr`) is
// skipped because the first `variable_list` child is `dot_index_expression`,
// not `identifier`.
//
// Lua has no generics, so `type_args_query` is empty.
// Lua has no idiomatic type-narrowing guard pattern, so `type_guard_query` is empty.
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static LUA_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "lua",

    // Matches the `assignment_statement` node present in both bare and local
    // forms. Captures:
    //   @lhs — the first identifier on the left of `=`
    //   @rhs — the first expression on the right
    //
    // Using `(identifier) @lhs` as the first named child of `variable_list`
    // skips table-field LHS (`t.k = expr`) because those use
    // `dot_index_expression`, not `identifier`.
    assignment_query: r#"
        (assignment_statement
            (variable_list
                (identifier) @lhs)
            (expression_list
                (_) @rhs))
    "#,

    // Lua has no common type-narrowing idiom expressible as a tree-sitter pattern.
    type_guard_query: "",

    // Lua has no call-site generic arguments.
    discriminant_guard_query: "",
    type_args_query: "",
    literal_type_kinds: &[],
};

pub const LUA_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
    crate::indexer::flow_cfg::CfgNodeKinds {
        function_kinds: &[
            "function_declaration",
            "local_function",
            "function_definition",
        ],
        block_kinds: &["block"],
        if_kind: "if_statement",
        if_consequence_field: "consequence",
        if_consequence_body: None,
        if_alternative_field: "alternative",
        if_alternative_body: None,
        if_condition_field: "condition",
        assignment_kind: "assignment_statement",
        assignment_lhs_field: "left",
        declarator_kind: "__lua_no_declarator__",
        declarator_name_field: "name",
        binding_name_kinds: &["identifier"],
        definition_name_kinds: &["identifier"],
        bare_return_name_kinds: &["identifier"],
        function_name_fields: &["name"],
        loop_kinds: &[
            "while_statement",
            "for_statement",
            "for_numeric_statement",
            "for_generic_statement",
            "repeat_statement",
        ],
        loop_body_field: "body",
        loop_condition_field: Some("condition"),
        switch_kinds: &[],
        switch_value_field: "value",
        switch_body_field: None,
        switch_case_kinds: &[],
        switch_default_kinds: &[],
        // `variable_declaration` is a thin wrapper over `assignment_statement`
        // for `local x = …`; recursing through it puts the assignment into the
        // current block where collect_defs_in sees it.
        transparent_kinds: &["variable_declaration"],
        implicit_return_candidate: None,
        condition_true_guard: None,
    };
