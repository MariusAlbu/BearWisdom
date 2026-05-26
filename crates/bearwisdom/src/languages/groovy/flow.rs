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
};
