// =============================================================================
// r_lang/flow.rs — R FlowConfig for local-variable type inference
//
// R uses `binary_operator` for all binary expressions. Assignment operators
// are distinguished by the `operator` field text: `<-`, `=`, `<<-`.
// Right-arrow (`->`) is skipped — it reverses operand order, so `@lhs`
// would capture the value expression and `@rhs` the variable name, which
// is the opposite of what the flow runner expects.
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static R_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "r",

    // `x <- <expr>` and `x = <expr>` — the two dominant R assignment forms.
    // `binary_operator` has named fields `lhs`, `operator`, and `rhs`.
    // The `#eq?` predicates guard against arithmetic and logical operators
    // that share the same node kind.
    assignment_query: r#"
        (binary_operator
            lhs: (identifier) @lhs
            operator: _ @op
            rhs: (_) @rhs
            (#match? @op "^(<-|=|<<-)$"))
    "#,

    // R has no standard structural type-narrowing construct analogous to
    // Python's `isinstance` or TypeScript's `instanceof` that produces a
    // scoped narrowing block.
    type_guard_query: "",

    // R has no call-site generic type arguments.
    discriminant_guard_query: "",
    type_args_query: "",
    literal_type_kinds: &[],
};
