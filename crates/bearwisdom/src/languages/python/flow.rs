// =============================================================================
// python/flow.rs — R5 Sprint 3 Python FlowConfig
//
// Python has no generics, so `type_args_query` is empty.
// =============================================================================

use crate::indexer::flow::FlowConfig;

/// Return-expression query for body-based return-type inference (INFER-3).
/// Captures the returned expression of every `return <expr>`;
/// `flow::run_return_query` resolves the owning function by ancestor-walk
/// (dropping a return whose nearest function is a nested `lambda`).
pub const PY_RETURN_QUERY: &str = r#"
    (return_statement (_) @return.expr)
"#;

pub static PY_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "python",

    // `x = <expr>` — Python's `assignment` node has `left` and `right` fields.
    // Annotated form `x: T = <expr>` uses the same assignment node with a
    // `type` field present. An attribute target on the instance receiver
    // (`self.x = <expr>`) names a declared member instead of a new local, so it
    // is captured as `@lhs.member`; the receiver predicate keeps assignments
    // through any other object out — those name members of a declaration this
    // file does not own.
    assignment_query: r#"
        (assignment
            left: (identifier) @lhs
            right: (_) @rhs)

        (assignment
            left: (attribute
                object: (identifier) @_recv
                attribute: (identifier) @lhs.member)
            right: (_) @rhs
            (#any-of? @_recv "self" "cls"))

        (typed_parameter
            (identifier) @lhs.param
            type: (type) @type)

        (typed_default_parameter
            name: (identifier) @lhs.param
            type: (type) @type)

        (default_parameter
            name: (identifier) @lhs.param)

        (parameters
            (identifier) @lhs.param)
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
    discriminant_guard_query: "",
    type_args_query: "",
    literal_type_kinds: &[],
};

pub const PYTHON_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
    crate::indexer::flow_cfg::CfgNodeKinds {
        function_kinds: &["function_definition", "lambda"],
        block_kinds: &["block"],
        if_kind: "if_statement",
        if_consequence_field: "consequence",
        if_consequence_body: None,
        if_alternative_field: "alternative",
        if_alternative_body: None,
        if_condition_field: "condition",
        assignment_kind: "assignment",
        assignment_lhs_field: "left",
        // Python has no separate declarator — assignment is the def.
        declarator_kind: "__python_no_declarator__",
        declarator_name_field: "name",
        binding_name_kinds: &["identifier"],
        definition_name_kinds: &["identifier"],
        bare_return_name_kinds: &["identifier"],
        function_name_fields: &["name"],
        loop_kinds: &["while_statement", "for_statement"],
        loop_body_field: "body",
        loop_condition_field: Some("condition"),
        // PEP 634 match — disabled by default; default `_` pattern subsumes else.
        switch_kinds: &[],
        switch_value_field: "subject",
        switch_body_field: Some("body"),
        switch_case_kinds: &["case_clause"],
        switch_default_kinds: &[],
        transparent_kinds: &[],
        implicit_return_candidate: None,
        condition_true_guard: None,
    };
