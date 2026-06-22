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
