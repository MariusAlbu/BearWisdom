// =============================================================================
// ruby/flow.rs — R5 Sprint 4 Ruby FlowConfig
//
// Ruby has no generics, so `type_args_query` is empty.
// =============================================================================

use crate::indexer::flow::FlowConfig;

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
            (#eq? @_m "is_a?"))
    "#,

    // Ruby has no call-site generic arguments.
    type_args_query: "",
};
