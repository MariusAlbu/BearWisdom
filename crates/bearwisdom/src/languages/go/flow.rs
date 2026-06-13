// =============================================================================
// go/flow.rs — R5 Sprint 4 Go FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;
use crate::types::{ExtractedSymbol, FlowMeta, Narrowing, SymbolKind};
use rustc_hash::FxHashMap;
use tree_sitter::Node;

/// Return-expression query for body-based return-type inference (INFER-3).
/// Captures the returned value of every `return`; `flow::run_return_query`
/// resolves the owning function by ancestor-walk (dropping a return whose
/// nearest function is a nested `func_literal`). Go wraps return values in an
/// `expression_list`; the single-value form is captured (multi-value returns
/// are a deeper sub-case).
pub const GO_RETURN_QUERY: &str = r#"
    (return_statement (expression_list (_) @return.expr))
"#;

pub static GO_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "go",

    // `x := <expr>` → `short_var_declaration`. `var x = <expr>` →
    // `var_declaration` + `var_spec`. Reassignment: `assignment_statement`.
    // Single-LHS form only (multi-value `a, b := …` is a deeper sub-case).
    //
    // The RHS is the EXPLICIT alternation of the chain-bearing node kinds
    // (call / selector / identifier / type-assertion / composite-literal)
    // rather than an unrestricted `(_)` wildcard. Tree-sitter compiles a `(_)`
    // inside an `(expression_list …)` into a query automaton that expands a
    // state per possible `_expression` production at that point; on certain Go
    // source shapes that combinatorial expansion allocated ~400MB and OOM'd.
    // Enumerating the node kinds the resolver actually consumes collapses the
    // automaton to a fixed, small state set.
    assignment_query: r#"
        (short_var_declaration
            left: (expression_list
                (identifier) @lhs)
            right: (expression_list
                [(call_expression)
                 (selector_expression)
                 (identifier)
                 (type_assertion_expression)
                 (composite_literal)] @rhs))

        (assignment_statement
            left: (expression_list
                (identifier) @lhs)
            right: (expression_list
                [(call_expression)
                 (selector_expression)
                 (identifier)
                 (type_assertion_expression)
                 (composite_literal)] @rhs))

        (var_spec
            name: (identifier) @lhs
            value: (expression_list
                [(call_expression)
                 (selector_expression)
                 (identifier)
                 (type_assertion_expression)
                 (composite_literal)] @rhs))
    "#,

    // Two Go narrowing forms:
    //   if v, ok := x.(Foo); ok { ... }     — type assertion, narrows `v` to Foo
    //   switch v := x.(type) { case *Foo: }  — type switch, narrows `v` per case
    // For the type switch, the alias `v` narrows to each case's type within that
    // case body (both the bare and pointer forms).
    type_guard_query: r#"
        (if_statement
            initializer: (short_var_declaration
                left: (expression_list
                    (identifier) @guard.local)
                right: (expression_list
                    (type_assertion_expression
                        type: (type_identifier) @guard.type)))
            consequence: (block) @guard.body)

        (type_switch_statement
            alias: (expression_list
                (identifier) @guard.local)
            (type_case
                [(type_identifier) @guard.type
                 (pointer_type (type_identifier) @guard.type)]) @guard.body)
    "#,

    // Expression-switch discriminant: `switch x.field { case "lit": ... }`
    // narrows `x` per case to the branch carrying that discriminant literal.
    // @guard.local is the scrutinee receiver, @guard.prop the selected field,
    // @guard.literal the case's string literal (one per case value),
    // @guard.body the case node whose range scopes the narrowing.
    discriminant_guard_query: r#"
        (expression_switch_statement
            value: (selector_expression
                operand: (identifier) @guard.local
                field: (field_identifier) @guard.prop)
            (expression_case
                value: (expression_list
                    [(interpreted_string_literal) @guard.literal
                     (raw_string_literal) @guard.literal])) @guard.body)
    "#,

    // Go's generic type-argument node structure varies between grammar
    // releases; leave empty in v1 to avoid compilation failures. The chain
    // walker already honors seg.type_args if extractors populate them.
    type_args_query: "",
};

// ---------------------------------------------------------------------------
// Range-element flow for table-driven anonymous-struct slices
// ---------------------------------------------------------------------------

/// Type a `range` value variable as the element type of a slice/array whose
/// elements are an anonymous struct.
///
/// The shape this targets:
/// ```go
/// tests := []struct{ name string; want int }{ ... }
/// for _, tc := range tests { tc.want }   // `tc.want` must bind to `want`
/// ```
///
/// The anonymous struct's fields are already indexed as members of the
/// ENCLOSING FUNCTION: the extractor files them under the function's qualified
/// name as their `scope_path`, so they register in the member index keyed by
/// that qname. The element type's identity is therefore the enclosing
/// function's qname — the same key the slice local `tests` carries as its own
/// `scope_path`. Typing `tc` with that qname routes `tc.<field>` through the
/// generic member lookup with no synthesized TypeId.
///
/// The binding is a `Narrowing` scoped to the loop body's byte range, not a
/// flat decl-type. Two functions in one file can each declare a `tc` whose
/// element is a different anonymous struct; a byte-range narrowing keeps each
/// `tc.<field>` resolving against its own loop body, where a flat name→type map
/// would have the second binding shadow the first for both reads.
///
/// Two structural hops, both O(nodes):
///   1. Find each slice/array `composite_literal` with a `struct_type` element
///      and correlate it to the declaration's LHS identifier — the slice local.
///      Record `slice_local_name -> enclosing-function qname`, read from that
///      local's `scope_path` symbol.
///   2. For each `range_clause` whose ranged expression is a bare identifier
///      naming a recorded slice local, emit a `Narrowing` over the enclosing
///      `for_statement`'s body that types the VALUE loop variable (the second
///      `left` identifier) as that qname.
pub(crate) fn bind_range_element_locals(
    root: &Node,
    src: &[u8],
    symbols: &[ExtractedSymbol],
    meta: &mut FlowMeta,
) {
    let mut slice_locals: FxHashMap<String, String> = FxHashMap::default();
    collect_struct_slice_locals(root, src, symbols, &mut slice_locals);
    if slice_locals.is_empty() {
        return;
    }
    narrow_value_vars_over_slice_locals(root, src, &slice_locals, meta);
}

/// Walk the tree collecting `(slice_local_name -> element type)` for every
/// declaration whose RHS is a slice/array composite literal of a struct
/// element. An anonymous `struct_type` element types as the enclosing-fn qname
/// (where its inline fields are filed); a named `type_identifier` element types
/// as the type's own name, resolved in scope like any other narrowing.
fn collect_struct_slice_locals(
    node: &Node,
    src: &[u8],
    symbols: &[ExtractedSymbol],
    out: &mut FxHashMap<String, String>,
) {
    if node.kind() == "composite_literal" {
        if let Some(typing) = slice_element_typing(node, src) {
            if let Some((name, line)) = decl_lhs_for_composite(node, src) {
                let ty = match typing {
                    SliceElemTyping::AnonStruct => {
                        scope_of_local(symbols, name, line).map(str::to_string)
                    }
                    SliceElemTyping::Named(t) => qualify_named_type(symbols, &t),
                };
                if let Some(ty) = ty {
                    out.insert(name.to_string(), ty);
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            collect_struct_slice_locals(&child, src, symbols, out);
        }
    }
}

/// How a slice/array `composite_literal`'s element types its range vars.
enum SliceElemTyping {
    /// Anonymous `struct_type` element — fields are filed under the enclosing
    /// function's qname.
    AnonStruct,
    /// Named `type_identifier` element — the type's own name.
    Named(String),
}

/// The element typing of a slice/array composite literal, or `None` when the
/// literal isn't a slice/array of a typable struct element.
///
/// The literal's type is the FIRST named child (positional — tree-sitter-go does
/// not expose it as a `type` field). An anonymous literal (`{1, 2}`) has a
/// `literal_value` as its first named child and no type, so it is rejected.
/// Qualified elements (`[]pkg.Case`) are not narrowed.
fn slice_element_typing(composite: &Node, src: &[u8]) -> Option<SliceElemTyping> {
    let ty = composite.named_child(0)?;
    let elem = match ty.kind() {
        // slice_type has no `element` field name in tree-sitter-go; its element
        // is the sole named child. array_type carries an `element` field.
        "slice_type" => first_named_child(&ty),
        "array_type" => ty.child_by_field_name("element"),
        _ => None,
    }?;
    match elem.kind() {
        "struct_type" => Some(SliceElemTyping::AnonStruct),
        "type_identifier" => Some(SliceElemTyping::Named(
            elem.utf8_text(src).ok()?.to_string(),
        )),
        _ => None,
    }
}

/// The LHS identifier and its line for the declaration that owns `composite`,
/// when the declaration is `name := <composite>` / `var name = <composite>`.
/// The composite must be the initializer value, not a nested element, so the
/// search stops at the nearest declaration ancestor and reads its LHS.
fn decl_lhs_for_composite<'a>(composite: &Node, src: &'a [u8]) -> Option<(&'a str, u32)> {
    let mut cur = composite.parent();
    while let Some(n) = cur {
        match n.kind() {
            "short_var_declaration" | "assignment_statement" => {
                return lhs_identifier(&n, "left", src);
            }
            "var_spec" => {
                return lhs_identifier(&n, "name", src);
            }
            // A declaration boundary is the nearest statement-level node; stop
            // climbing past a block so a composite nested inside another literal
            // isn't mis-attributed to an outer declaration.
            "block" | "source_file" => return None,
            _ => cur = n.parent(),
        }
    }
    None
}

/// The first identifier of a declaration's name/left field, with its line.
/// For `name := …` the field is an `expression_list`; for `var name = …` it is
/// the `name` identifier directly.
fn lhs_identifier<'a>(decl: &Node, field: &str, src: &'a [u8]) -> Option<(&'a str, u32)> {
    let target = decl.child_by_field_name(field)?;
    let ident = if target.kind() == "identifier" {
        target
    } else {
        first_named_child_of_kind(&target, "identifier")?
    };
    let text = ident.utf8_text(src).ok()?;
    if text.is_empty() || text == "_" {
        return None;
    }
    Some((text, ident.start_position().row as u32))
}

/// For every `range_clause` whose ranged expression is a bare identifier naming
/// a recorded slice local, emit a `Narrowing` over the enclosing for-statement's
/// body that types the value loop variable as the slice's element qname.
fn narrow_value_vars_over_slice_locals(
    node: &Node,
    src: &[u8],
    slice_locals: &FxHashMap<String, String>,
    meta: &mut FlowMeta,
) {
    if node.kind() == "range_clause" {
        if let Some((var_name, scope)) = range_value_var(node, src, slice_locals) {
            if let Some(body) = for_body_for_range(node) {
                meta.narrowings.push(Narrowing {
                    name: var_name.to_string(),
                    narrowed_type: scope.to_string(),
                    byte_start: body.start_byte() as u32,
                    byte_end: body.end_byte() as u32,
                });
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            narrow_value_vars_over_slice_locals(&child, src, slice_locals, meta);
        }
    }
}

/// `(value_var_name, element_qname)` for a `range_clause` `for k, v := range
/// slice` whose `slice` is a recorded slice local. The value variable is the
/// SECOND `left` identifier (`v`); the form `for v := range …` over a slice
/// binds `v` to the index, not the element, so it is rejected.
fn range_value_var<'a, 'b>(
    range_clause: &Node,
    src: &'a [u8],
    slice_locals: &'b FxHashMap<String, String>,
) -> Option<(&'a str, &'b str)> {
    let right = range_clause.child_by_field_name("right")?;
    if right.kind() != "identifier" {
        return None;
    }
    let slice_name = right.utf8_text(src).ok()?;
    let scope = slice_locals.get(slice_name)?;

    let left = range_clause.child_by_field_name("left")?;
    let idents: Vec<Node> = {
        let mut c = left.walk();
        left.children(&mut c)
            .filter(|n| n.kind() == "identifier")
            .collect()
    };
    // Element binding needs both the index and the value variable present; a
    // single-variable range yields the index, not the element.
    let value = idents.get(1)?;
    let text = value.utf8_text(src).ok()?;
    if text.is_empty() || text == "_" {
        return None;
    }
    Some((text, scope.as_str()))
}

/// The body `block` of the `for_statement` that owns `range_clause`. The
/// narrowing scopes to this range so the value-var type holds only inside the
/// loop body and cannot collide with a same-named value var in another loop.
fn for_body_for_range<'a>(range_clause: &Node<'a>) -> Option<Node<'a>> {
    let for_stmt = range_clause.parent()?;
    let mut cursor = for_stmt.walk();
    let found = for_stmt.children(&mut cursor).find(|c| c.kind() == "block");
    found
}

/// The qualified name of the in-file type declaration named `name` — the
/// member index keys struct fields under the package-qualified type name, so a
/// bare element name must qualify before it can type a range var. `None` when
/// no in-file type-like declaration carries the name (primitive elements,
/// types declared in other files) — the range var is left un-narrowed rather
/// than typed to an unresolvable name.
fn qualify_named_type(symbols: &[ExtractedSymbol], name: &str) -> Option<String> {
    symbols
        .iter()
        .find(|s| {
            s.name == name
                && matches!(
                    s.kind,
                    SymbolKind::Struct | SymbolKind::Class | SymbolKind::Interface
                )
        })
        .map(|s| s.qualified_name.clone())
}

/// The `scope_path` of the local variable named `name` declared at or before
/// `line` — the nearest preceding declaration, mirroring the assignment query's
/// name + nearest-line correlation.
fn scope_of_local<'a>(symbols: &'a [ExtractedSymbol], name: &str, line: u32) -> Option<&'a str> {
    symbols
        .iter()
        .filter(|s| s.name == name && s.start_line <= line)
        .max_by_key(|s| s.start_line)
        .and_then(|s| s.scope_path.as_deref())
}

fn first_named_child<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.is_named());
    found
}

fn first_named_child_of_kind<'a>(node: &Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|c| c.is_named() && c.kind() == kind);
    found
}

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;
