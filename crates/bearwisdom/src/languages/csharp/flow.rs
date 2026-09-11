// =============================================================================
// csharp/flow.rs — R5 Sprint 4 C# FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

/// Return-expression query for body-based return-type inference (INFER-3).
/// Captures the returned expression of every `return <expr>`;
/// `flow::run_return_query` resolves the owning method by ancestor-walk
/// (dropping a return whose nearest function is a nested `lambda_expression`
/// or `local_function_statement`).
pub const CSHARP_RETURN_QUERY: &str = r#"
    (return_statement (_) @return.expr)
"#;

pub static CSHARP_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "csharp",

    // `var x = foo();` / `Foo x = foo();` / reassignment `x = foo();`.
    // tree-sitter-c-sharp uses `variable_declarator` with `name` and
    // `value` fields, and `assignment_expression` for reassignment.
    assignment_query: r#"
        (variable_declarator
            name: (identifier) @lhs
            (_) @rhs)

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)

        (parameter
            type: (_) @type
            name: (identifier) @lhs.param)

        ((variable_declaration
            type: (_) @type
            (variable_declarator
                name: (identifier) @lhs))
          (#not-eq? @type "var"))
    "#,

    // C# pattern-matching narrowing. Two forms:
    //   if (x is Foo)   — `x` narrows to Foo in the block (constant_pattern)
    //   if (x is Foo f) — the binding `f` is typed Foo in the block; the
    //                     declaration_pattern's children are (type, binding).
    type_guard_query: r#"
        (if_statement
            condition: (is_pattern_expression
                (identifier) @guard.local
                (constant_pattern
                    (identifier) @guard.type))
            consequence: (block) @guard.body)

        (if_statement
            condition: (is_pattern_expression
                (declaration_pattern
                    (identifier) @guard.type
                    (identifier) @guard.local))
            consequence: (block) @guard.body)
    "#,

    // Generic method invocation: `repo.FindOne<User>()` — `generic_name`
    // holds the identifier and type_argument_list.
    discriminant_guard_query: "",
    type_args_query: r#"
        (invocation_expression
            function: (member_access_expression
                name: (generic_name
                    (identifier) @call.method
                    (type_argument_list
                        (identifier) @call.type_arg))))
    "#,
    literal_type_kinds: &[],
};

pub const CSHARP_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
    crate::indexer::flow_cfg::CfgNodeKinds {
        function_kinds: &[
            "method_declaration",
            "constructor_declaration",
            "local_function_statement",
            "lambda_expression",
        ],
        block_kinds: &["block"],
        if_kind: "if_statement",
        if_consequence_field: "consequence",
        if_consequence_body: None,
        if_alternative_field: "alternative",
        if_alternative_body: None,
        if_condition_field: "condition",
        assignment_kind: "assignment_expression",
        assignment_lhs_field: "left",
        declarator_kind: "variable_declarator",
        declarator_name_field: "name",
        binding_name_kinds: &["identifier"],
        definition_name_kinds: &["identifier"],
        bare_return_name_kinds: &["identifier"],
        function_name_fields: &["name"],
        loop_kinds: &[
            "while_statement",
            "for_statement",
            "for_each_statement",
            "do_statement",
        ],
        loop_body_field: "body",
        loop_condition_field: Some("condition"),
        switch_kinds: &["switch_statement"],
        switch_value_field: "value",
        switch_body_field: Some("body"),
        switch_case_kinds: &["switch_section"],
        switch_default_kinds: &[],
        transparent_kinds: &[],
        implicit_return_candidate: None,
        condition_true_guard: None,
    };
