// =============================================================================
// scala/flow.rs — R5 Sprint 4 Scala FlowConfig
// =============================================================================

use crate::indexer::flow::{correlate_scala_rhs_ref, FlowConfig, TUPLE_INDEX_KEY_PREFIX};
use crate::types::{ExtractedRef, ExtractedSymbol, FlowMeta};
use tree_sitter::Node;

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

/// Attach each direct `case (left, right) => ...` binding to the value being
/// matched.  This deliberately accepts only a flat tuple of two-or-more
/// identifiers; richer patterns need recursive/extractor semantics and must
/// not be flattened into positional tuple projections.
///
/// The second product is a case-only lexical graph for reads in each arm.
/// Scala has only callback lexical identity today, so a plain name-keyed cache
/// would otherwise let sibling arms sharing `left` exchange inferred types.
pub(crate) fn bind_match_tuple_cases(
    root: &Node,
    src: &[u8],
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
    meta: &mut FlowMeta,
) {
    let mut graph = crate::indexer::lexical::LexicalBindings::default();
    let root_scope = graph.add_scope(None, root.start_byte() as u32, root.end_byte() as u32, true);
    let mut stack = vec![*root];
    while let Some(node) = stack.pop() {
        if node.kind() == "match_expression" && !has_match_ancestor(node) {
            bind_match_tuple_case(node, src, symbols, refs, meta, &mut graph, root_scope);
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        // `scope_at` expects scope children in source order. The LIFO walk
        // therefore pushes them in reverse so the next pop is left-to-right.
        stack.extend(children.into_iter().rev());
    }
    if !graph.references.is_empty() || !graph.symbols.is_empty() {
        meta.case_lexical = Some(graph);
    }
}

fn bind_match_tuple_case(
    expression: Node,
    src: &[u8],
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
    meta: &mut FlowMeta,
    graph: &mut crate::indexer::lexical::LexicalBindings,
    parent_scope: crate::indexer::lexical::ScopeId,
) {
    let Some(value) = expression
        .child_by_field_name("value")
        .or_else(|| expression.child_by_field_name("scrutinee"))
    else {
        return;
    };
    let value_ref = correlate_scala_rhs_ref(refs, &value);
    let Some(body) = expression.child_by_field_name("body") else {
        return;
    };
    let mut cursor = body.walk();
    for clause in body.named_children(&mut cursor) {
        if clause.kind() != "case_clause" {
            continue;
        }
        let scope = graph.add_scope(
            Some(parent_scope),
            clause.start_byte() as u32,
            clause.end_byte() as u32,
            false,
        );
        let nested_matches = direct_nested_matches(clause);
        let mut excluded_ranges: Vec<_> = nested_matches
            .iter()
            .filter_map(|nested| nested.child_by_field_name("body"))
            .map(|body| (body.start_byte() as u32, body.end_byte() as u32))
            .collect();
        excluded_ranges.extend(nested_function_ranges(clause));
        let pattern = clause
            .child_by_field_name("pattern")
            .filter(|pattern| pattern.kind() == "tuple_pattern");
        let bindings = pattern.and_then(|pattern| {
            let mut pattern_cursor = pattern.walk();
            let bindings: Vec<_> = pattern.named_children(&mut pattern_cursor).collect();
            (bindings.len() >= 2
                && bindings
                    .iter()
                    .all(|binding| binding.kind() == "identifier"))
            .then_some(bindings)
        });

        for (position, binding) in bindings.into_iter().flatten().enumerate() {
            let Ok(name) = binding.utf8_text(src) else {
                continue;
            };
            let Some(symbol) = symbols.iter().position(|symbol| {
                symbol.byte_offset == binding.start_byte() as u32 && symbol.name == name
            }) else {
                continue;
            };
            // Scala's general local-scope graph has not migrated yet. If this
            // arm declares the same spelling again, abstain from the outer
            // case binding instead of assigning its tuple type to the shadow.
            // Declarations inside nested match bodies are owned by the nested
            // case scopes and do not shadow reads claimed by this arm.
            if case_arm_redeclares_name(clause, name, src, &excluded_ranges) {
                continue;
            }
            let name_id = graph.intern(name);
            let binding_id = graph.declare(scope, name_id, binding.start_byte() as u32, None);
            graph.attach_symbol(symbol, binding_id);

            // A call ref starts at its callee, which is also the root of a
            // member chain (`left.use()` starts at `left`). Restricting both
            // the clause range and root spelling makes the case owner exact.
            for reference in refs.iter().filter(|reference| {
                reference.byte_offset >= clause.start_byte() as u32
                    && reference.byte_offset < clause.end_byte() as u32
                    && !excluded_ranges.iter().any(|(start, end)| {
                        reference.byte_offset >= *start && reference.byte_offset < *end
                    })
                    && reference
                        .chain
                        .as_ref()
                        .and_then(|chain| chain.segments.first())
                        .is_some_and(|segment| segment.name == name)
            }) {
                graph.references.insert(reference.byte_offset, binding_id);
            }

            if let Some(ref_idx) = value_ref {
                meta.flow_binding_destructure
                    .entry(ref_idx)
                    .or_default()
                    .push((symbol, format!("{TUPLE_INDEX_KEY_PREFIX}{position}")));
            }
        }

        for nested in nested_matches {
            bind_match_tuple_case(nested, src, symbols, refs, meta, graph, scope);
        }
    }
}

/// Nested matches need a case scope parented on their enclosing arm. Until the
/// Scala graph models that hierarchy, neither arm claims the other's reads.
fn has_match_ancestor(node: Node) -> bool {
    let mut parent = node.parent();
    while let Some(current) = parent {
        if current.kind() == "match_expression" {
            return true;
        }
        parent = current.parent();
    }
    false
}

fn direct_nested_matches(clause: Node) -> Vec<Node> {
    let mut matches = Vec::new();
    let mut stack = Vec::new();
    let mut cursor = clause.walk();
    stack.extend(clause.named_children(&mut cursor));
    while let Some(node) = stack.pop() {
        if node.kind() == "match_expression" {
            matches.push(node);
            continue;
        }
        if is_function_boundary(node.kind()) {
            continue;
        }
        let mut children = node.walk();
        stack.extend(node.named_children(&mut children));
    }
    matches.sort_by_key(|node| node.start_byte());
    matches
}

fn nested_function_ranges(clause: Node) -> Vec<(u32, u32)> {
    let mut ranges = Vec::new();
    let mut stack = Vec::new();
    let mut cursor = clause.walk();
    stack.extend(clause.named_children(&mut cursor));
    while let Some(node) = stack.pop() {
        if is_function_boundary(node.kind()) {
            ranges.push((node.start_byte() as u32, node.end_byte() as u32));
            continue;
        }
        let mut children = node.walk();
        stack.extend(node.named_children(&mut children));
    }
    ranges
}

fn is_function_boundary(kind: &str) -> bool {
    matches!(
        kind,
        "function_definition" | "function_declaration" | "lambda_expression"
    )
}

fn case_arm_redeclares_name(
    clause: Node,
    name: &str,
    src: &[u8],
    nested_bodies: &[(u32, u32)],
) -> bool {
    let mut stack = Vec::new();
    let mut cursor = clause.walk();
    stack.extend(clause.named_children(&mut cursor));
    while let Some(node) = stack.pop() {
        let start = node.start_byte() as u32;
        if nested_bodies
            .iter()
            .any(|(body_start, body_end)| start >= *body_start && start < *body_end)
        {
            continue;
        }
        if matches!(
            node.kind(),
            "val_definition" | "var_definition" | "val_declaration" | "var_declaration"
        ) {
            let declared = node.child_by_field_name("name").or_else(|| {
                let mut children = node.walk();
                let declared = node
                    .named_children(&mut children)
                    .find(|child| child.kind() == "identifier");
                declared
            });
            if declared
                .and_then(|identifier| identifier.utf8_text(src).ok())
                .is_some_and(|declared| declared == name)
            {
                return true;
            }
            continue;
        }
        let mut children = node.walk();
        stack.extend(node.named_children(&mut children));
    }
    false
}

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

    #[test]
    fn direct_tuple_case_arms_keep_same_spelled_bindings_source_addressed() {
        let source = r#"
object O {
  def decode() = make() match {
    case (value, other) => value.left()
    case (other, value) => value.right()
  }
}
"#;
        let (symbols, flow) = flow_for(source);
        let left = source.find("value.left").expect("first case use") as u32;
        let right = source.find("value.right").expect("second case use") as u32;
        let graph = flow
            .case_lexical
            .as_ref()
            .expect("direct case tuple bindings build identity graph");
        let left_binding = graph
            .references
            .get(&left)
            .expect("left case root is source-addressed");
        let right_binding = graph
            .references
            .get(&right)
            .expect("right case root is source-addressed");
        assert_ne!(
            left_binding, right_binding,
            "same-spelled bindings in sibling cases must remain distinct"
        );

        let tuple_entries = flow
            .flow_binding_destructure
            .values()
            .flatten()
            .filter(|(symbol, _)| symbols[*symbol].name == "value")
            .map(|(_, key)| key.as_str())
            .collect::<Vec<_>>();
        assert!(tuple_entries.contains(&"$tuple:0"));
        assert!(tuple_entries.contains(&"$tuple:1"));
    }

    #[test]
    fn case_binding_abstains_when_the_arm_redeclares_the_same_name() {
        let source = r#"
object O {
  def decode() = make() match {
    case (value, other) =>
      val value = replacement()
      value.right()
  }
}
"#;
        let (symbols, flow) = flow_for(source);
        let shadowed_use = source.find("value.right").expect("shadowed use") as u32;
        assert!(
            flow.case_lexical
                .as_ref()
                .is_none_or(|graph| !graph.references.contains_key(&shadowed_use)),
            "the outer case binding must not claim an inner local shadow"
        );
        let case_value = source.find("(value, other)").unwrap() + 1;
        assert!(
            flow.flow_binding_destructure
                .values()
                .flatten()
                .all(|(symbol, _)| symbols[*symbol].byte_offset != case_value as u32),
            "a shadowed case slot must fail closed instead of receiving the outer tuple type"
        );
    }

    #[test]
    fn case_binding_does_not_claim_a_nested_function_parameter_shadow() {
        let source = r#"
object O {
  def decode() = make() match {
    case (value, other) =>
      def local(value: Second): Unit = value.second()
      value.first()
  }
}
"#;
        let (_, flow) = flow_for(source);
        let nested_use = source.find("value.second").expect("nested function use") as u32;
        let outer_use = source.find("value.first").expect("outer case use") as u32;
        let graph = flow.case_lexical.as_ref().expect("case identity graph");
        assert!(
            !graph.references.contains_key(&nested_use),
            "a nested function parameter belongs to its function, not the case arm"
        );
        assert!(
            graph.references.contains_key(&outer_use),
            "the case binding remains available after the local function"
        );
    }

    #[test]
    fn nested_direct_tuple_cases_receive_child_identity_scopes() {
        let source = r#"
object O {
  def decode() = outer() match {
    case (outerLeft, outerRight) => inner() match {
      case (innerLeft, innerRight) => innerLeft.use()
    }
  }
}
"#;
        let (symbols, flow) = flow_for(source);
        let inner_read = source.find("innerLeft.use").expect("inner use") as u32;
        let graph = flow
            .case_lexical
            .as_ref()
            .expect("nested case identity graph");
        assert!(graph.references.contains_key(&inner_read));
        assert!(
            flow.flow_binding_destructure
                .values()
                .flatten()
                .any(|(symbol, key)| { symbols[*symbol].name == "innerLeft" && key == "$tuple:0" }),
            "the nested flat case must project its first tuple position"
        );
    }
}
