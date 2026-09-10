// =============================================================================
// scala/flow.rs — R5 Sprint 4 Scala FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

/// Return-expression query for body-based return-type inference (INFER-3).
/// Captures the returned expression of every explicit `return e`
/// (`return_expression`), plus the concise expression body of `def f = e` via
/// `@return.tail` (the `function_definition` body field). `flow::run_return_query`
/// resolves the owning def by ancestor-walk (dropping a return whose nearest
/// function is a nested lambda), and skips a `@return.tail` whose kind is a
/// `block` — `def f = { … }` returns from its block's final expression, which
/// the structural tail-of-block pass attributes (gated on
/// `CfgNodeKinds.block_tail_returns`), not the block node itself.
pub const SCALA_RETURN_QUERY: &str = r#"
    (return_expression (_) @return.expr)
    (function_definition body: (_) @return.tail)
"#;

pub static SCALA_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "scala",

    // `val/var x = <expr>` — `val_definition`/`var_definition` in
    // tree-sitter-scala. Captures the name pattern and value expression.
    assignment_query: r#"
        (val_definition
            pattern: (identifier) @lhs
            value: (_) @rhs)

        (var_definition
            pattern: (identifier) @lhs
            value: (_) @rhs)

        (val_definition
            pattern: (tuple_pattern
                (identifier) @destruct.bind)
            value: (_) @rhs)

        (var_definition
            pattern: (tuple_pattern
                (identifier) @destruct.bind)
            value: (_) @rhs)

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)

        (parameter
            name: (identifier) @lhs.param
            type: (_) @type)

        (binding
            name: (identifier) @lhs.param
            type: (_) @type)

        (binding
            name: (identifier) @lhs.param)

        (lambda_expression
            parameters: (identifier) @lhs.param)

        (val_definition
            pattern: (identifier) @lhs
            type: (_) @type)
    "#,

    // Pattern-match narrowing via `case Foo(_) =>` is too general to query
    // cleanly. v1 leaves this empty; Scala's strong inference already
    // surfaces types via declared_type on pattern bindings.
    type_guard_query: "",

    // `repo.findOne[User]()` — Scala type arguments on calls.
    discriminant_guard_query: "",
    type_args_query: r#"
        (generic_function
            function: (field_expression
                field: (identifier) @call.method)
            type_arguments: (type_arguments
                (type_identifier) @call.type_arg))
    "#,
    literal_type_kinds: &[],
};

#[cfg(test)]
mod tests {
    use super::SCALA_FLOW_CONFIG;
    use crate::indexer::flow::{run_flow_queries, BindingSymbols};
    use crate::languages::scala::{extract, ScalaPlugin};
    use crate::languages::LanguagePlugin;

    fn flow_for(source: &str) -> (Vec<crate::types::ExtractedSymbol>, crate::types::FlowMeta) {
        let mut extracted = extract::extract(source);
        let grammar = ScalaPlugin.grammar("scala").expect("Scala grammar");
        let flow = run_flow_queries(
            source,
            &grammar,
            &SCALA_FLOW_CONFIG,
            &mut extracted.symbols,
            &mut extracted.refs,
            BindingSymbols::Synthesize,
        );
        (extracted.symbols, flow)
    }

    #[test]
    fn direct_tuple_val_and_var_bind_each_identifier_to_their_rhs_positions() {
        for keyword in ["val", "var"] {
            let source = format!("object O {{ def f = {{ {keyword} (key, inputs) = make() }} }}");
            let (symbols, flow) = flow_for(&source);
            let key_idx = symbols
                .iter()
                .position(|symbol| symbol.name == "key")
                .expect("key symbol");
            let inputs_idx = symbols
                .iter()
                .position(|symbol| symbol.name == "inputs")
                .expect("inputs symbol");
            let entries: Vec<_> = flow.flow_binding_destructure.values().flatten().collect();

            assert!(
                entries
                    .iter()
                    .any(|(index, key)| *index == key_idx && key == "$tuple:0"),
                "{keyword}: {entries:?}"
            );
            assert!(
                entries
                    .iter()
                    .any(|(index, key)| *index == inputs_idx && key == "$tuple:1"),
                "{keyword}: {entries:?}"
            );
        }
    }

    #[test]
    fn non_direct_tuple_patterns_do_not_record_tuple_projections() {
        for source in [
            "object O { def f = { val ((key, inputs), tail) = make() } }",
            "object O { def f = { val (key: Key, inputs) = make() } }",
            "object O { def f = { val (_, inputs) = make() } }",
            "object O { def f = { val (Pair(key), inputs) = make() } }",
            "object O { def f(x: Any) = x match { case (key, inputs) => make() } }",
        ] {
            let (_, flow) = flow_for(source);
            assert!(
                flow.flow_binding_destructure.is_empty(),
                "non-direct tuple pattern must abstain: {source}: {:?}",
                flow.flow_binding_destructure
            );
        }
    }
}
