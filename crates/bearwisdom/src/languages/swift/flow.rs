// =============================================================================
// swift/flow.rs — Swift FlowConfig (CFG-native `is T` / `as? T` / `if let`
// flow promotion).
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static SWIFT_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "swift",

    // `let x = <expr>` / `var x = <expr>` parse as `property_declaration` with
    // a `value` field; the bound name is a `pattern` child. Reassignment is
    // `assignment` with `target`/`result`. Single-LHS forms only.
    assignment_query: r#"
        (property_declaration
            name: (pattern (simple_identifier) @lhs)
            value: (_) @rhs)

        (assignment
            target: (directly_assignable_expression
                (simple_identifier) @lhs)
            result: (_) @rhs)
    "#,

    // Two Swift narrowing forms, both attaching the guard to the then-block
    // (`statements`) by byte range — the if_statement lists its branches
    // positionally with no consequence field:
    //   if x is T { ... }            — `check_expression` narrows `x` to T
    //   if let y = x as? T { ... }   — optional downcast binds `y` as T
    type_guard_query: r#"
        (if_statement
            (check_expression
                (simple_identifier) @guard.local
                (user_type (type_identifier) @guard.type))
            (statements) @guard.body)

        (if_statement
            (simple_identifier) @guard.local
            (as_expression
                (user_type (type_identifier) @guard.type))
            (statements) @guard.body)
    "#,

    discriminant_guard_query: "",
    type_args_query: "",
};
