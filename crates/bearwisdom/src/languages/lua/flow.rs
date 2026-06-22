// =============================================================================
// lua/flow.rs — Lua FlowConfig for local-var type binding
//
// Lua has two assignment forms that this config covers:
//
//   1. Global/bare:  `x = expr`
//      Tree-sitter:  assignment_statement
//                      variable_list → identifier @lhs
//                      expression_list → _ @rhs
//
//   2. Local:        `local x = expr`
//      Tree-sitter:  variable_declaration
//                      assignment_statement   ← inner node
//                        variable_list → identifier @lhs
//                        expression_list → _ @rhs
//
// The `assignment_query` matches the inner `assignment_statement` in both
// cases. The outer `variable_declaration` wrapper is transparent — the inner
// node is a direct child of `variable_declaration` and the query cursor
// descends into it naturally.
//
// Multi-assignment (`a, b = f()`) binds only the first name — the RHS
// byte range correlates to whichever ref falls inside it, which is correct
// for the common single-name form. Table field LHS (`t.k = expr`) is
// skipped because the first `variable_list` child is `dot_index_expression`,
// not `identifier`.
//
// Lua has no generics, so `type_args_query` is empty.
// Lua has no idiomatic type-narrowing guard pattern, so `type_guard_query` is empty.
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static LUA_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "lua",

    // Matches the `assignment_statement` node present in both bare and local
    // forms. Captures:
    //   @lhs — the first identifier on the left of `=`
    //   @rhs — the first expression on the right
    //
    // Using `(identifier) @lhs` as the first named child of `variable_list`
    // skips table-field LHS (`t.k = expr`) because those use
    // `dot_index_expression`, not `identifier`.
    assignment_query: r#"
        (assignment_statement
            (variable_list
                (identifier) @lhs)
            (expression_list
                (_) @rhs))
    "#,

    // Lua has no common type-narrowing idiom expressible as a tree-sitter pattern.
    type_guard_query: "",

    // Lua has no call-site generic arguments.
    discriminant_guard_query: "",
    type_args_query: "",
    literal_type_kinds: &[],
};
