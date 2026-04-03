// =============================================================================
// go/calls.rs  —  Call and reference extraction for Go
// =============================================================================

use super::helpers::node_text;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Body traversal — refs + local variable symbols
// ---------------------------------------------------------------------------

/// Walk a function/method body, extracting both:
///   1. All call/composite-literal/type-assertion refs (via `extract_refs_from_body`)
///   2. Local variable symbols from `:=` declarations and `for range` clauses
///
/// `enclosing_idx` is the index of the enclosing function/method symbol.
/// `qualified_prefix` is the qualified name of the enclosing function (used as
/// the scope_path for the emitted Variable symbols).
pub(super) fn extract_body_with_symbols(
    body: &Node,
    source: &str,
    enclosing_idx: usize,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    extract_body_with_symbols_inner(body, source, enclosing_idx, qualified_prefix, symbols, refs);
}

fn extract_body_with_symbols_inner(
    node: &Node,
    source: &str,
    enclosing_idx: usize,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `:=` short variable declaration
            "short_var_declaration" => {
                super::symbols::extract_short_var_decl(
                    &child,
                    source,
                    symbols,
                    refs,
                    Some(enclosing_idx),
                    qualified_prefix,
                    enclosing_idx,
                );
                // Don't recurse further — extract_short_var_decl handles its RHS.
            }

            // `for i, v := range slice { ... }`
            "for_statement" => {
                extract_for_range_vars(&child, source, enclosing_idx, qualified_prefix, symbols, refs);
                // Recurse into the body block.
                let mut fc = child.walk();
                for fc_child in child.children(&mut fc) {
                    if fc_child.kind() == "block" {
                        extract_body_with_symbols_inner(
                            &fc_child, source, enclosing_idx, qualified_prefix, symbols, refs,
                        );
                    }
                }
                // Also extract plain refs from the whole for_statement.
                extract_refs_from_body(&child, source, enclosing_idx, refs);
            }

            // `select { case msg := <-ch: ... }` — variables in communication_case
            "select_statement" => {
                let mut sc = child.walk();
                for case_child in child.children(&mut sc) {
                    if case_child.kind() == "communication_case" {
                        // Look for a short_var_declaration inside the case header.
                        let mut cc = case_child.walk();
                        for cc_child in case_child.children(&mut cc) {
                            if cc_child.kind() == "short_var_declaration" {
                                super::symbols::extract_short_var_decl(
                                    &cc_child,
                                    source,
                                    symbols,
                                    refs,
                                    Some(enclosing_idx),
                                    qualified_prefix,
                                    enclosing_idx,
                                );
                            }
                        }
                        // Recurse into case body.
                        extract_body_with_symbols_inner(
                            &case_child, source, enclosing_idx, qualified_prefix, symbols, refs,
                        );
                    } else if case_child.kind() == "default_case" {
                        extract_body_with_symbols_inner(
                            &case_child, source, enclosing_idx, qualified_prefix, symbols, refs,
                        );
                    }
                }
                // Also extract plain refs.
                extract_refs_from_body(&child, source, enclosing_idx, refs);
            }

            // All other nodes: extract refs and recurse for nested symbols.
            _ => {
                extract_refs_from_body(&child, source, enclosing_idx, refs);
                extract_body_with_symbols_inner(
                    &child, source, enclosing_idx, qualified_prefix, symbols, refs,
                );
            }
        }
    }
}

/// Extract loop variables from `for i, v := range slice { ... }`.
///
/// Tree-sitter-go shape:
/// ```text
/// for_statement
///   for_clause / for_range_clause
///     left:  expression_list   → identifiers
///     right: expression        → the slice/map/channel
///   block
/// ```
///
/// `for_range_clause` has `left` and `right` field names in the grammar.
fn extract_for_range_vars(
    for_node: &Node,
    source: &str,
    enclosing_idx: usize,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    use super::helpers::{go_visibility, qualify, scope_from_prefix};

    let mut cursor = for_node.walk();
    for child in for_node.children(&mut cursor) {
        if child.kind() != "range_clause" {
            continue;
        }

        let left = match child.child_by_field_name("left") {
            Some(n) => n,
            None => continue,
        };

        // Collect identifiers from the left side.
        let mut lc = left.walk();
        for ident in left.children(&mut lc) {
            if ident.kind() != "identifier" {
                continue;
            }
            let name = node_text(&ident, source);
            if name == "_" {
                continue;
            }
            let qualified_name = qualify(&name, qualified_prefix);
            let visibility = go_visibility(&name);

            symbols.push(ExtractedSymbol {
                name,
                qualified_name,
                kind: crate::types::SymbolKind::Variable,
                visibility,
                start_line: ident.start_position().row as u32,
                end_line: ident.end_position().row as u32,
                start_col: ident.start_position().column as u32,
                end_col: ident.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: scope_from_prefix(qualified_prefix),
                parent_index: Some(enclosing_idx),
            });
        }

        // Extract refs from the right-hand side (the range expression).
        if let Some(right) = child.child_by_field_name("right") {
            extract_refs_from_body(&right, source, enclosing_idx, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// Body reference extraction (calls, instantiations)
// ---------------------------------------------------------------------------

pub(super) fn extract_refs_from_body(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                extract_call_ref(&child, source, source_symbol_index, refs);
                // Recurse into arguments for nested calls.
                let mut acursor = child.walk();
                for arg_child in child.children(&mut acursor) {
                    if arg_child.kind() == "argument_list" {
                        extract_refs_from_body(
                            &arg_child,
                            source,
                            source_symbol_index,
                            refs,
                        );
                    }
                }
            }
            "composite_literal" => {
                extract_composite_literal_ref(&child, source, source_symbol_index, refs);
                // Recurse into body for nested composites / calls.
                let mut bcursor = child.walk();
                for body_child in child.children(&mut bcursor) {
                    if body_child.kind() == "literal_value" {
                        extract_refs_from_body(
                            &body_child,
                            source,
                            source_symbol_index,
                            refs,
                        );
                    }
                }
            }

            // `x.(*Admin)` — type assertion
            "type_assertion_expression" => {
                extract_type_assertion_ref(&child, source, source_symbol_index, refs);
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }

            // `switch v := x.(type) { case *Admin: ... }`
            "type_switch_statement" => {
                extract_type_switch_refs(&child, source, source_symbol_index, refs);
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }

            // `string(bytes)`, `int64(x)` — type conversion expression.
            // The `type` field is the target type; emit a TypeRef for it.
            // Also recurse into the operand expression for nested calls.
            "type_conversion_expression" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    let type_name = super::helpers::extract_go_type_name(&type_node, source);
                    if !type_name.is_empty()
                        && !super::helpers::is_go_builtin_type(&type_name)
                    {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: type_name,
                            kind: EdgeKind::TypeRef,
                            line: type_node.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }

            // `ch <- value` — send statement: recurse into value
            "send_statement" => {
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }

            // `select { case msg := <-ch: ... }` — recurse into all case bodies
            "select_statement" => {
                extract_select_refs(&child, source, source_symbol_index, refs);
            }

            // `go doWork()` / `go func() { ... }()` — extract calls inside the goroutine.
            // The wildcard would recurse, but we name it explicitly so it's clear
            // and to ensure the call_expression inside is fully processed.
            "go_statement" | "defer_statement" => {
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }

            // `func() { ... }` — anonymous function literal.
            // TypeRefs for parameter types in the func_literal's parameter_list.
            "func_literal" => {
                extract_func_literal_type_refs(&child, source, source_symbol_index, refs);
                // Recurse into the body block for nested calls.
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }

            // `[N]Foo` — array type used as a value expression (e.g. in composite literals).
            "array_type" => {
                let type_name = super::helpers::extract_go_type_name(&child, source);
                if !type_name.is_empty() && !super::helpers::is_go_builtin_type(&type_name) {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: type_name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }

            // `func(A) B` — function type in an expression position.
            "function_type" => {
                super::helpers::extract_function_type_refs(
                    &child, source, source_symbol_index, refs,
                );
            }

            // `List[int]` — generic type (Go 1.18+).
            "generic_type" => {
                let type_name = super::helpers::extract_go_type_name(&child, source);
                if !type_name.is_empty() && !super::helpers::is_go_builtin_type(&type_name) {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: type_name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
                // Also recurse into type arguments for their contained type refs.
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }

            _ => {
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }
        }
    }
}

/// Extract TypeRef edges for parameter types of a `func_literal` node.
///
/// `func_literal` children: `func` (keyword), `parameter_list`, `result?`, `block`
fn extract_func_literal_type_refs(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "parameter_list" => {
                // Walk parameter declarations.
                let mut pc = child.walk();
                for param in child.children(&mut pc) {
                    if param.kind() != "parameter_declaration"
                        && param.kind() != "variadic_parameter_declaration"
                    {
                        continue;
                    }
                    // The type is the last named child that isn't an identifier.
                    let type_node = (0..param.child_count())
                        .filter_map(|i| param.child(i))
                        .filter(|c| c.is_named() && c.kind() != "identifier")
                        .last();
                    if let Some(tn) = type_node {
                        let name = super::helpers::extract_go_type_name(&tn, source);
                        if !name.is_empty() && !super::helpers::is_go_builtin_type(&name) {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: tn.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                }
            }
            "result" => {
                // Return type(s).
                let mut rc = child.walk();
                for ret_child in child.children(&mut rc) {
                    if !ret_child.is_named() {
                        continue;
                    }
                    let name = super::helpers::extract_go_type_name(&ret_child, source);
                    if !name.is_empty() && !super::helpers::is_go_builtin_type(&name) {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::TypeRef,
                            line: ret_child.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

/// Recurse into each `communication_case` body inside a `select_statement`.
///
/// Tree-sitter-go shape:
/// ```text
/// select_statement
///   communication_case
///     send_statement / receive_statement / ...
///     (body statements)
///   default_case
///     (body statements)
/// ```
fn extract_select_refs(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "communication_case" | "default_case" => {
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }
            _ => {}
        }
    }
}

/// Emit a `Calls` ref for a `call_expression`.
///
/// `call_expression` children (positional):
///   function (identifier | selector_expression | ...), argument_list
///
/// For `bar.Baz()` the function part is a `selector_expression` with children:
///   operand, `.`, `field_identifier`
///
/// Special case: `make(chan User, 10)` — emit a TypeRef for the channel element
/// type in addition to the normal Calls edge.
fn extract_call_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The function part is the first named child (use index to avoid cursor borrow).
    let func_node = match node.named_child(0) {
        Some(n) => n,
        None => return,
    };

    let func_name = node_text(&func_node, source);

    // `make(chan T, ...)` — extract the channel element type as a TypeRef.
    if func_name == "make" {
        extract_make_chan_type_ref(node, source, source_symbol_index, refs);
    }

    // Build a structured chain for selector expressions; fall back to the
    // existing single-name extraction for bare identifiers.
    let chain = build_chain(func_node, source);

    let target_name = chain
        .as_ref()
        .and_then(|c| c.segments.last())
        .map(|s| s.name.clone())
        .unwrap_or_else(|| match func_node.kind() {
            "selector_expression" => (0..func_node.named_child_count())
                .filter_map(|i| func_node.named_child(i))
                .find(|c| c.kind() == "field_identifier")
                .map(|n| node_text(&n, source))
                .unwrap_or_else(|| node_text(&func_node, source)),
            _ => func_name.clone(),
        });

    if target_name.is_empty() {
        return;
    }

    // When the callee is a bare identifier that starts with an uppercase letter,
    // Go convention says it is exported — this may be a user-defined type
    // conversion (`MyString(b)` is syntactically a call_expression in tree-sitter-go,
    // not a type_conversion_expression).  Emit a TypeRef so the resolution engine
    // can treat it as a potential type usage.
    if func_node.kind() == "identifier"
        && target_name.chars().next().map_or(false, |c| c.is_uppercase())
        && !super::helpers::is_go_builtin_type(&target_name)
    {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target_name.clone(),
            kind: EdgeKind::TypeRef,
            line: func_node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }

    crate::parser::extractors::emit_chain_type_ref(&chain, source_symbol_index, &func_node, refs);
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name,
        kind: EdgeKind::Calls,
        line: func_node.start_position().row as u32,
        module: None,
        chain,
    });
}

/// For `make(chan User, 10)` emit a TypeRef to `User` (the channel element type).
///
/// Tree-sitter-go shape:
/// ```text
/// call_expression
///   identifier "make"
///   argument_list
///     channel_type
///       type_identifier "User"
///     int_literal "10"
/// ```
fn extract_make_chan_type_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let args = match (0..node.named_child_count())
        .filter_map(|i| node.named_child(i))
        .find(|c| c.kind() == "argument_list")
    {
        Some(a) => a,
        None => return,
    };

    // First argument to make() — look for a channel_type node.
    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        if child.kind() == "channel_type" {
            // channel_type children: `chan` (anon), element_type
            let mut inner = child.walk();
            for elem in child.children(&mut inner) {
                if !elem.is_named() {
                    continue; // skip `chan` keyword
                }
                let elem_name = go_type_node_name(&elem, source);
                if !elem_name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: elem_name,
                        kind: EdgeKind::TypeRef,
                        line: elem.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
                break;
            }
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Member chain builder
// ---------------------------------------------------------------------------

/// Build a structured `MemberChain` from a Go function/selector node.
///
/// Go uses `selector_expression` for member access (not `member_expression`):
///
/// `repo.FindOne()`:
/// ```text
/// selector_expression
///   identifier "repo"
///   field_identifier "FindOne"
/// ```
///
/// `s.repo.FindOne()`:
/// ```text
/// selector_expression
///   selector_expression
///     identifier "s"
///     field_identifier "repo"
///   field_identifier "FindOne"
/// ```
///
/// Returns `None` for bare `identifier` nodes (single-segment — handled by
/// the existing scope-chain strategies) and for any node we can't walk.
pub(super) fn build_chain(node: Node, source: &str) -> Option<MemberChain> {
    // Only build a chain for multi-segment expressions.
    if node.kind() == "identifier" {
        return None;
    }
    let mut segments = Vec::new();
    build_chain_inner(node, source, &mut segments)?;
    if segments.len() < 2 {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, source: &str, segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "identifier" => {
            segments.push(ChainSegment {
                name: node_text(&node, source),
                node_kind: "identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "selector_expression" => {
            // Children (by index): operand, `.` (anon), field_identifier
            // We need the first named child (operand) and the last named child
            // (field_identifier).  Use indexed access to avoid cursor re-borrow.
            let named_count = node.named_child_count();
            if named_count < 2 {
                return None;
            }
            let operand = node.named_child(0)?;
            let field = node.named_child(named_count - 1)?;

            // Recurse into the operand to build the prefix chain.
            build_chain_inner(operand, source, segments)?;

            segments.push(ChainSegment {
                name: node_text(&field, source),
                node_kind: field.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "call_expression" => {
            // Nested call in a chain: `a.B().C()` — walk into its function child.
            let func = node.named_child(0)?;
            build_chain_inner(func, source, segments)
        }

        // Unknown node — can't build a chain from this.
        _ => None,
    }
}

/// Emit an `Instantiates` ref for a `composite_literal`.
///
/// `composite_literal` children: type (identifier or qualified_type), literal_value
pub(super) fn extract_composite_literal_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The type is the first named child (use index to avoid cursor borrow).
    let type_node = match node.named_child(0) {
        Some(n) => n,
        None => return,
    };

    // Skip if the first named child is the literal_value `{...}` (happens for
    // anonymous composite literals like `{1, 2}`).
    if type_node.kind() == "literal_value" {
        return;
    }

    let type_name = match type_node.kind() {
        "type_identifier" => node_text(&type_node, source),
        "qualified_type" => {
            // `pkg.TypeName` — find the last `type_identifier` by index.
            let last_ti = (0..type_node.named_child_count())
                .filter_map(|i| type_node.named_child(i))
                .filter(|c| c.kind() == "type_identifier")
                .last();
            match last_ti {
                Some(n) => node_text(&n, source),
                None => node_text(&type_node, source),
            }
        }
        _ => node_text(&type_node, source),
    };

    if type_name.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: type_name,
        kind: EdgeKind::Instantiates,
        line: type_node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// Type narrowing — type assertions and type switches
// ---------------------------------------------------------------------------

/// Emit a TypeRef for `x.(*Admin)` — a `type_assertion_expression`.
///
/// Tree-sitter-go structure:
/// ```text
/// type_assertion_expression
///   identifier "x"          ← operand
///   pointer_type / type_identifier / qualified_type   ← asserted type
/// ```
/// The asserted type is the last named child.
pub(super) fn extract_type_assertion_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let named_count = node.named_child_count();
    if named_count < 2 {
        return;
    }
    let type_node = match node.named_child(named_count - 1) {
        Some(n) => n,
        None => return,
    };

    let type_name = go_type_node_name(&type_node, source);
    if type_name.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: type_name,
        kind: EdgeKind::TypeRef,
        line: type_node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

/// Emit TypeRefs for each case type in a `type_switch_statement`.
///
/// ```go
/// switch v := x.(type) {
///     case *Admin:   ...
///     case *User:    ...
/// }
/// ```
/// Tree-sitter-go: `type_switch_statement` → `type_case` children,
/// each with a `type` field (or positional type children).
pub(super) fn extract_type_switch_refs(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_case" {
            // Each case clause can list multiple types: `case *Foo, *Bar:`
            // Walk all children for type nodes.
            let mut inner = child.walk();
            for type_child in child.children(&mut inner) {
                match type_child.kind() {
                    "type_identifier" | "pointer_type" | "qualified_type" => {
                        let name = go_type_node_name(&type_child, source);
                        if !name.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: type_child.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Extract a simple type name from a Go type node, dereferencing pointer types.
fn go_type_node_name(node: &Node, source: &str) -> String {
    match node.kind() {
        "type_identifier" => node_text(node, source),
        "pointer_type" => {
            // `*Admin` — the named child is the underlying type.
            node.named_child(0)
                .map(|n| go_type_node_name(&n, source))
                .unwrap_or_default()
        }
        "qualified_type" => {
            // `pkg.Admin` — use the last type_identifier.
            (0..node.named_child_count())
                .filter_map(|i| node.named_child(i))
                .filter(|c| c.kind() == "type_identifier")
                .last()
                .map(|n| node_text(&n, source))
                .unwrap_or_else(|| node_text(node, source))
        }
        _ => String::new(),
    }
}
