// =============================================================================
// python/flow.rs — R5 Sprint 3 Python FlowConfig
//
// Python has no generics, so `type_args_query` is empty.
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static PY_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "python",

    // `x = <expr>` — Python's `assignment` node has `left` and `right` fields.
    // Annotated form `x: T = <expr>` uses the same assignment node with a
    // `type` field present.
    assignment_query: r#"
        (assignment
            left: (identifier) @lhs
            right: (_) @rhs)
    "#,

    // `if isinstance(x, Derived): ...` — the canonical Python narrowing
    // pattern. Captures the type identifier (second arg) and the body.
    type_guard_query: r#"
        (if_statement
            condition: (call
                function: (identifier) @_fn
                arguments: (argument_list
                    (identifier) @guard.local
                    (identifier) @guard.type))
            consequence: (block) @guard.body
            (#eq? @_fn "isinstance"))
    "#,

    // Python has no call-site generic arguments.
    type_args_query: "",
};
