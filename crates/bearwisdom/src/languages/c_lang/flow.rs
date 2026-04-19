// =============================================================================
// c_lang/flow.rs — R5 Sprint 4 C/C++ FlowConfig
//
// C has no runtime type narrowing; C++ has `dynamic_cast` but it's awkward to
// query structurally. v1 ships assignment + C++ template type args; narrowing
// stays empty.
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static C_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "c",

    // `Type x = <expr>;` → `declaration` holds an `init_declarator`.
    // Reassignment: `assignment_expression`.
    assignment_query: r#"
        (init_declarator
            declarator: (identifier) @lhs
            value: (_) @rhs)

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)
    "#,

    // No reliable, cheap narrowing query for C/C++. Skip.
    type_guard_query: "",

    // C++ template args require tree-sitter-cpp (not tree-sitter-c); we use
    // tree-sitter-c for `.c`/`.h` files where templates don't apply. Leave
    // this empty — cross-dialect query support is future work.
    type_args_query: "",
};
