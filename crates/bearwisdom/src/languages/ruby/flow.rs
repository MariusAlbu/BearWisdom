// =============================================================================
// ruby/flow.rs — R5 Sprint 4 Ruby FlowConfig
//
// Ruby has no generics, so `type_args_query` is empty.
// =============================================================================

use crate::indexer::flow::FlowConfig;

/// Return-expression query for body-based return-type inference (INFER-3).
/// Captures the returned expression of every explicit `return e` (the `return`
/// node wraps its value in an `argument_list`); `flow::run_return_query`
/// resolves the owning method by ancestor-walk (dropping a return whose nearest
/// function is a nested block/lambda). Ruby's dominant implicit
/// last-expression return has no return node — it is handled by the structural
/// tail-of-block pass, gated on `CfgNodeKinds.block_tail_returns`.
pub const RUBY_RETURN_QUERY: &str = r#"
    (return (argument_list (_) @return.expr))
"#;

pub static RUBY_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "ruby",

    // `x = <expr>` — Ruby uses `assignment` node with `left` and `right`.
    assignment_query: r#"
        (assignment
            left: (identifier) @lhs
            right: (_) @rhs)
    "#,

    // `if x.is_a?(Foo) then ... end` — Ruby's narrowing idiom. Capture the
    // body as the `then` block; the narrowing holds while `x` is checked.
    //
    // Matches:
    //   (if
    //     condition: (call
    //       receiver: (identifier) @guard.local
    //       method: (identifier = "is_a?")
    //       arguments: (argument_list (constant) @guard.type))
    //     consequence: (then) @guard.body)
    type_guard_query: r#"
        (if
            condition: (call
                receiver: (identifier) @guard.local
                method: (identifier) @_m
                arguments: (argument_list
                    (constant) @guard.type))
            consequence: (then) @guard.body
            (#any-of? @_m "is_a?" "kind_of?"))
    "#,

    // Ruby has no call-site generic arguments.
    discriminant_guard_query: "",
    type_args_query: "",
};
