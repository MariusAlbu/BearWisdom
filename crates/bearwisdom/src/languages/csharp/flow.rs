// =============================================================================
// csharp/flow.rs — R5 Sprint 4 C# FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

/// Return-expression query for body-based return-type inference (INFER-3).
/// Captures the returned expression of every `return <expr>`;
/// `flow::run_return_query` resolves the owning method by ancestor-walk
/// (dropping a return whose nearest function is a nested `lambda_expression`
/// or `local_function_statement`).
pub const CSHARP_RETURN_QUERY: &str = r#"
    (return_statement (_) @return.expr)
"#;

pub static CSHARP_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "csharp",

    // `var x = foo();` / `Foo x = foo();` / reassignment `x = foo();`.
    // tree-sitter-c-sharp uses `variable_declarator` with `name` and
    // `value` fields, and `assignment_expression` for reassignment.
    assignment_query: r#"
        (variable_declarator
            name: (identifier) @lhs
            (_) @rhs)

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)
    "#,

    // C# pattern-matching narrowing. Two forms:
    //   if (x is Foo)   — `x` narrows to Foo in the block (constant_pattern)
    //   if (x is Foo f) — the binding `f` is typed Foo in the block; the
    //                     declaration_pattern's children are (type, binding).
    type_guard_query: r#"
        (if_statement
            condition: (is_pattern_expression
                (identifier) @guard.local
                (constant_pattern
                    (identifier) @guard.type))
            consequence: (block) @guard.body)

        (if_statement
            condition: (is_pattern_expression
                (declaration_pattern
                    (identifier) @guard.type
                    (identifier) @guard.local))
            consequence: (block) @guard.body)
    "#,

    // Generic method invocation: `repo.FindOne<User>()` — `generic_name`
    // holds the identifier and type_argument_list.
    discriminant_guard_query: "",
    type_args_query: r#"
        (invocation_expression
            function: (member_access_expression
                name: (generic_name
                    (identifier) @call.method
                    (type_argument_list
                        (identifier) @call.type_arg))))
    "#,
};
