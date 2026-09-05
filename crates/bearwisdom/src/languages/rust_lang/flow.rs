// =============================================================================
// rust_lang/flow.rs — R5 Sprint 3 Rust FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

/// Return-expression query for body-based return-type inference (INFER-3).
/// Captures the returned expression of every explicit `return e`;
/// `flow::run_return_query` resolves the owning function by ancestor-walk
/// (dropping a return whose nearest function is a nested `closure_expression`).
/// Rust's dominant tail-expression form (`fn f() -> T { e }`) has no
/// `return_expression` node — it is handled by the structural tail-of-block
/// pass, gated on `CfgNodeKinds.block_tail_returns`, not by this query.
pub const RUST_RETURN_QUERY: &str = r#"
    (return_expression (_) @return.expr)
"#;

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

        (parameter
            pattern: (identifier) @lhs.param
            type: (_) @type)

        (closure_parameters
            (identifier) @lhs.param)
    "#,

    // Type guards in Rust are done via `if let Some(x) = ...`, `match`, and
    // `as` casts. v1 keeps this empty — narrowings require pattern analysis
    // beyond a simple tree-sitter query, and Rust's strong type system
    // already drives precise types through the extractor's declared_type.
    type_guard_query: "",

    // Turbofish: `foo::<T>()`, `Vec::<String>::new()`. The extractor emits
    // the call ref on the `generic_function` node; the chain segment is the
    // method/function name (`foo`, `new`). Capture the turbofish type args.
    discriminant_guard_query: "",
    type_args_query: r#"
        (call_expression
            function: (generic_function
                function: (_) @call.method
                type_arguments: (type_arguments
                    (type_identifier) @call.type_arg)))
    "#,
    literal_type_kinds: &[],
};
