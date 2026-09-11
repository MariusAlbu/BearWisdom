// =============================================================================
// dart/flow.rs — Dart FlowConfig (CFG-native `is T` flow promotion).
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static DART_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "dart",

    // `var x = <expr>` / `final x = <expr>` parse as
    // `initialized_variable_definition` with `name` + `value` fields;
    // reassignment is `assignment_expression` with `left`/`right`. Single-LHS
    // forms only.
    assignment_query: r#"
        (initialized_variable_definition
            name: (identifier) @lhs
            value: (_) @rhs)

        (assignment_expression
            left: (assignable_expression
                (identifier) @lhs)
            right: (_) @rhs)

        (formal_parameter
            (type_identifier) @type
            name: (identifier) @lhs.param)

        (formal_parameter
            name: (identifier) @lhs.param)

        (formal_parameter
            (identifier) @lhs.param)

        (initialized_variable_definition
            (type_identifier) @type
            name: (identifier) @lhs)
    "#,

    // `if (x is Foo) { ... }` promotes `x` to `Foo` in the then-block. The
    // condition is the positional `type_test_expression` child of the
    // if_statement; the then-block is the `consequence` field.
    type_guard_query: r#"
        (if_statement
            (type_test_expression
                (identifier) @guard.local
                (type_test (type_identifier) @guard.type))
            consequence: (block) @guard.body)
    "#,

    discriminant_guard_query: "",
    type_args_query: "",
    literal_type_kinds: &[],
};

pub const DART_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
    crate::indexer::flow_cfg::CfgNodeKinds {
        function_kinds: &["function_body", "function_expression_body"],
        block_kinds: &["block"],
        if_kind: "if_statement",
        if_consequence_field: "consequence",
        if_consequence_body: None,
        if_alternative_field: "alternative",
        if_alternative_body: None,
        // No condition field on Dart's if_statement; the type-test narrowing rides
        // the type_guard_query, so an empty field name disables the syntactic path.
        if_condition_field: "",
        assignment_kind: "assignment_expression",
        assignment_lhs_field: "left",
        declarator_kind: "initialized_variable_definition",
        declarator_name_field: "name",
        binding_name_kinds: &["identifier"],
        definition_name_kinds: &["identifier"],
        bare_return_name_kinds: &["identifier"],
        function_name_fields: &["name"],
        loop_kinds: &["while_statement", "for_statement", "do_statement"],
        loop_body_field: "body",
        loop_condition_field: Some("condition"),
        switch_kinds: &["switch_statement"],
        switch_value_field: "condition",
        switch_body_field: Some("body"),
        switch_case_kinds: &["switch_statement_case"],
        switch_default_kinds: &["switch_statement_default"],
        transparent_kinds: &[],
        implicit_return_candidate: None,
        condition_true_guard: None,
    };
