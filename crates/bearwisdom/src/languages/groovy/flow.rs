use crate::indexer::flow::FlowConfig;

pub static GROOVY_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "groovy",

    // `Type x = <expr>` and `def x = <expr>` both parse as
    // `local_variable_declaration` with a `variable_declarator` child
    // carrying `name` and `value` fields. Bare reassignment `x = rhs`
    // is an `assignment_expression` with `left`/`right` fields.
    assignment_query: r#"
        (local_variable_declaration
            declarator: (variable_declarator
                name: (identifier) @lhs
                value: (_) @rhs))

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)
    "#,

    // `if (x instanceof Foo) { ... }` — Groovy's `instanceof_expression`
    // uses `left` for the tested value and `right` for the type.
    // The consequence may be a block or a single statement; `(_)` accepts both.
    type_guard_query: r#"
        (if_statement
            condition: (parenthesized_expression
                (instanceof_expression
                    left: (identifier) @guard.local
                    right: (_) @guard.type))
            consequence: (_) @guard.body)
    "#,

    // Groovy does not have call-site type arguments in the Java
    // `obj.<T>method()` style; leave this empty.
    discriminant_guard_query: "",
    type_args_query: "",
    literal_type_kinds: &[],
};

pub const GROOVY_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
    crate::indexer::flow_cfg::CfgNodeKinds {
        function_kinds: &["method_declaration", "constructor_declaration"],
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
        loop_kinds: &["while_statement", "for_statement", "do_statement"],
        loop_body_field: "body",
        loop_condition_field: Some("condition"),
        switch_kinds: &["switch_expression", "switch_statement"],
        switch_value_field: "condition",
        switch_body_field: Some("body"),
        switch_case_kinds: &["switch_block_statement_group"],
        switch_default_kinds: &[],
        transparent_kinds: &[],
        implicit_return_candidate: None,
        condition_true_guard: None,
    };
