// =============================================================================
// rust_lang/flow.rs — R5 Sprint 3 Rust FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static RUST_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "rust",

    // `let x = <expr>;` — tree-sitter-rust `let_declaration` has `pattern`,
    // `type`, and `value` fields. The binding pattern is a bare `identifier`
    // for `let x` and a `mut_pattern` for `let mut x`; both shapes bind the
    // same name. The `type:` field carries an explicit annotation
    // (`let x: T = …`), which types the local without resolving the
    // initializer. Reassignment `x = expr` uses `assignment_expression`.
    assignment_query: r#"
        (let_declaration
            pattern: [(identifier) @lhs (mut_pattern (identifier) @lhs)]
            value: (try_expression) @rhs_unwrap)

        (let_declaration
            pattern: [(identifier) @lhs (mut_pattern (identifier) @lhs)]
            value: (_) @rhs)

        (let_declaration
            pattern: [(identifier) @lhs (mut_pattern (identifier) @lhs)]
            type: (_) @type)

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)
    "#,

    // Type guards in Rust are done via `if let Some(x) = ...`, `match`, and
    // `as` casts. v1 keeps this empty — narrowings require pattern analysis
    // beyond a simple tree-sitter query, and Rust's strong type system
    // already drives precise types through the extractor's declared_type.
    type_guard_query: "",

    // Turbofish: `foo::<T>()`, `Vec::<String>::new()`. The extractor emits
    // the call ref on the `generic_function` node; the chain segment is the
    // method/function name (`foo`, `new`). Capture the turbofish type args.
    type_args_query: r#"
        (call_expression
            function: (generic_function
                function: (_) @call.method
                type_arguments: (type_arguments
                    (type_identifier) @call.type_arg)))
    "#,
};
