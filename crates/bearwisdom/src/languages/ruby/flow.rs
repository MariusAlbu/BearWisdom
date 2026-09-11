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
    literal_type_kinds: &[],
};

pub const RUBY_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
    crate::indexer::flow_cfg::CfgNodeKinds {
        function_kinds: &["method", "singleton_method", "block", "do_block"],
        block_kinds: &["body_statement", "then", "do"],
        if_kind: "if",
        if_consequence_field: "consequence",
        if_consequence_body: None,
        if_alternative_field: "alternative",
        if_alternative_body: None,
        if_condition_field: "condition",
        assignment_kind: "assignment",
        assignment_lhs_field: "left",
        declarator_kind: "__ruby_no_declarator__",
        declarator_name_field: "name",
        binding_name_kinds: &["identifier"],
        definition_name_kinds: &["identifier"],
        bare_return_name_kinds: &["identifier"],
        function_name_fields: &["name"],
        loop_kinds: &["while", "until", "while_modifier", "until_modifier"],
        loop_body_field: "body",
        loop_condition_field: Some("condition"),
        switch_kinds: &["case"],
        switch_value_field: "value",
        switch_body_field: None,
        switch_case_kinds: &["when"],
        switch_default_kinds: &["else"],
        transparent_kinds: &[],
        // A Ruby method returns its body's final expression with no `return`.
        implicit_return_candidate: Some(implicit_return_candidate),
        condition_true_guard: None,
    };
fn implicit_return_candidate(node: tree_sitter::Node) -> bool {
    let kind = node.kind();
    !(kind.ends_with("_statement")
        || kind.ends_with("_declaration")
        || kind.ends_with("_definition")
        || kind.contains("return")
        || kind == "block")
}
