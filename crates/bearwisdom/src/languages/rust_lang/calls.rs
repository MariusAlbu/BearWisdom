// =============================================================================
// rust/calls.rs  —  Call and use-declaration extraction for Rust
// =============================================================================

use super::helpers::node_text;
use super::patterns;
use super::symbols::{extract_method_from_fn, is_rust_primitive};
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// impl block handling
// ---------------------------------------------------------------------------

/// Process an `impl_item` — the container for methods.
/// The implementing type name becomes the qualified prefix for its methods.
///
/// Emits:
///   - A `Namespace`-kind symbol at the `impl_item` line (coverage signal for
///     the symbol_node_kinds list; represents the impl block as a scope container).
///   - An `Implements` edge when the form is `impl Trait for Type`.
///   - A `TypeRef` to the implementing type (coverage signal for ref_node_kinds).
///   - Attributes on the impl_item processed via `extract_decorators`.
pub(super) fn extract_impl(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    outer_prefix: &str,
) {
    let type_node = match node.child_by_field_name("type") {
        Some(n) => n,
        None => return,
    };
    let type_name = node_text(&type_node, source);

    // Emit a Namespace symbol at the impl_item line.  This gives the coverage
    // system something to match against for `impl_item` in symbol_node_kinds.
    let impl_sym_idx = symbols.len();
    {
        use super::helpers::{qualify, scope_from_prefix};
        let impl_name = if outer_prefix.is_empty() {
            type_name.clone()
        } else {
            format!("{outer_prefix}.{type_name}")
        };
        symbols.push(ExtractedSymbol {
            name: type_name.clone(),
            qualified_name: impl_name,
            kind: SymbolKind::Namespace,
            visibility: super::helpers::detect_visibility(node),
            start_line: node.start_position().row as u32,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: node.end_position().column as u32,
            signature: Some(format!("impl {type_name}")),
            doc_comment: None,
            scope_path: scope_from_prefix(outer_prefix),
            parent_index: None,
        });

        // TypeRef to the implementing type — coverage signal for ref_node_kinds.
        if !is_rust_primitive(&type_name) {
            refs.push(ExtractedRef {
                source_symbol_index: impl_sym_idx,
                target_name: type_name.clone(),
                kind: EdgeKind::TypeRef,
                line: type_node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
    }

    // Process attributes on the impl_item itself.
    super::decorators::extract_decorators(node, source, impl_sym_idx, refs);

    // `impl Trait for Type` — emit an Implements edge from the implementing type
    // back to the trait.  The trait name lives in the `trait` field.
    if let Some(trait_node) = node.child_by_field_name("trait") {
        let trait_name = rust_type_node_name(&trait_node, source);
        if !trait_name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: impl_sym_idx,
                target_name: trait_name,
                kind: EdgeKind::Implements,
                line: trait_node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
    }

    let impl_prefix = if outer_prefix.is_empty() {
        type_name
    } else {
        format!("{outer_prefix}.{type_name}")
    };

    // Process type_parameters and where_clause on the impl_item itself.
    // e.g. `impl<T: Clone> Foo<T>` or `impl<T> Bar where T: Send`.
    {
        let mut nc = node.walk();
        for nc_child in node.children(&mut nc) {
            match nc_child.kind() {
                "type_parameters" => {
                    patterns::extract_type_param_bounds(&nc_child, source, impl_sym_idx, refs);
                }
                "where_clause" => {
                    patterns::extract_where_clause(&nc_child, source, impl_sym_idx, refs);
                }
                _ => {}
            }
        }
    }

    let body = match node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(sym) = extract_method_from_fn(&child, source, None, &impl_prefix) {
                    let idx = symbols.len();
                    symbols.push(sym);
                    // Emit TypeRefs for parameter/return types in the signature.
                    super::symbols::extract_fn_signature_type_refs(&child, source, idx, refs);
                    {
                        let mut wc = child.walk();
                        for gc in child.children(&mut wc) {
                            match gc.kind() {
                                "type_parameters" => {
                                    patterns::extract_type_param_bounds(&gc, source, idx, refs);
                                }
                                "where_clause" => {
                                    patterns::extract_where_clause(&gc, source, idx, refs);
                                }
                                _ => {}
                            }
                        }
                    }
                    if let Some(fn_body) = child.child_by_field_name("body") {
                        extract_calls_from_body_with_symbols(&fn_body, source, idx, refs, Some(symbols));
                    }
                }
            }

            // `type Output = String;` — associated type in an impl or trait body.
            // Emit a TypeAlias symbol scoped to the impl type and a TypeRef for
            // the right-hand type (when it's a named type, not a primitive).
            "associated_type" | "type_item" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(&name_node, source);
                    if !name.is_empty() {
                        use super::helpers::{qualify, scope_from_prefix};
                        use crate::types::SymbolKind;
                        let qualified_name = qualify(&name, &impl_prefix);
                        let sym_idx = symbols.len();
                        symbols.push(crate::types::ExtractedSymbol {
                            name: name.clone(),
                            qualified_name,
                            kind: SymbolKind::TypeAlias,
                            visibility: None,
                            start_line: child.start_position().row as u32,
                            end_line: child.end_position().row as u32,
                            start_col: child.start_position().column as u32,
                            end_col: child.end_position().column as u32,
                            signature: Some(format!("type {name}")),
                            doc_comment: None,
                            scope_path: scope_from_prefix(&impl_prefix),
                            parent_index: None,
                        });
                        // Emit TypeRef if the RHS type is a named type.
                        if let Some(ty_node) = child.child_by_field_name("type") {
                            let type_name = rust_type_node_name(&ty_node, source);
                            if !type_name.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index: sym_idx,
                                    target_name: type_name,
                                    kind: EdgeKind::TypeRef,
                                    line: ty_node.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                });
                            }
                        }
                    }
                }
            }

            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

/// Recursively scan a function/method body for `call_expression` nodes
/// and emit `Calls` references.
pub(super) fn extract_calls_from_body(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    extract_calls_from_body_with_symbols(node, source, source_symbol_index, refs, None);
}

/// Variant that also emits Variable symbols for closure parameters.
pub(super) fn extract_calls_from_body_with_symbols(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    mut symbols: Option<&mut Vec<ExtractedSymbol>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Match arms: extract patterns (TypeRef for variants, Variable for bindings)
            "match_expression" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    patterns::extract_match_patterns(&child, source, source_symbol_index, syms, refs);
                    extract_calls_from_body_with_symbols(&child, source, source_symbol_index, refs, Some(syms));
                } else {
                    let mut tmp: Vec<ExtractedSymbol> = Vec::new();
                    patterns::extract_match_patterns(&child, source, source_symbol_index, &mut tmp, refs);
                    extract_calls_from_body(&child, source, source_symbol_index, refs);
                }
            }

            // tree-sitter-rust represents `if let Pat = val` as `if_expression` containing
            // a `let_condition` child (NOT as `if_let_expression`).
            // The `let_condition` holds: `let` keyword, pattern, `=`, value expression.
            "let_condition" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    patterns::extract_let_condition_pattern(
                        &child,
                        source,
                        source_symbol_index,
                        syms,
                        refs,
                    );
                    // Recurse into the condition (calls in the RHS value expression)
                    extract_calls_from_body_with_symbols(
                        &child,
                        source,
                        source_symbol_index,
                        refs,
                        Some(syms),
                    );
                } else {
                    let mut tmp: Vec<ExtractedSymbol> = Vec::new();
                    patterns::extract_let_condition_pattern(
                        &child,
                        source,
                        source_symbol_index,
                        &mut tmp,
                        refs,
                    );
                    extract_calls_from_body(&child, source, source_symbol_index, refs);
                }
            }

            // `println!()`, `vec![]`, `format!()`, custom macros.
            // Can't expand them, but we emit a Calls edge for the macro name.
            "macro_invocation" => {
                if let Some(macro_node) = child.child_by_field_name("macro") {
                    let name = node_text(&macro_node, source);
                    // Strip trailing `!` if present (some grammars include it).
                    let name = name.trim_end_matches('!').to_string();
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: macro_node.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                // Recurse into the token-tree arguments for nested calls inside the macro.
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, source, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, source, source_symbol_index, refs);
                }
            }

            // `x as u64` — type cast expression.  Emit TypeRef for the target type.
            "type_cast_expression" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    let type_name = rust_type_node_name(&type_node, source);
                    if !type_name.is_empty() {
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
                // Recurse into the value expression for nested calls.
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, source, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, source, source_symbol_index, refs);
                }
            }

            // `let x: T = expr;` — emit a Variable symbol for the binding pattern,
            // a TypeRef for the explicit type annotation (if any), and recurse
            // into the value expression for nested calls.
            "let_declaration" => {
                // Emit TypeRef for the declared type: `let x: MyType = ...`
                if let Some(type_node) = child.child_by_field_name("type") {
                    super::symbols::extract_type_refs_from_type_node(
                        &type_node,
                        source,
                        source_symbol_index,
                        refs,
                    );
                }
                if let Some(syms) = symbols.as_deref_mut() {
                    // Reuse the pattern extractor — handles identifiers, tuple patterns, etc.
                    // `let_declaration` and `let_condition` share the same `pattern` field.
                    super::patterns::extract_let_condition_pattern(
                        &child,
                        source,
                        source_symbol_index,
                        syms,
                        refs,
                    );
                    extract_calls_from_body_with_symbols(&child, source, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, source, source_symbol_index, refs);
                }
            }

            // `Point { x: 1, y: 2 }` — struct literal / constructor call.
            // Emit a Calls edge for the struct name so it appears in call hierarchy.
            "struct_expression" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = rust_type_node_name(&name_node, source);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name.clone(),
                            kind: EdgeKind::Calls,
                            line: name_node.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                        // Also emit TypeRef so the type graph is connected.
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::TypeRef,
                            line: name_node.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, source, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, source, source_symbol_index, refs);
                }
            }

            "call_expression" => {
                if let Some(func) = child.child_by_field_name("function") {
                    let chain = build_chain(func, source);

                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| {
                            let callee_text = node_text(&func, source);
                            callee_text
                                .rsplit("::")
                                .next()
                                .unwrap_or(&callee_text)
                                .rsplit('.')
                                .next()
                                .unwrap_or(&callee_text)
                                .trim()
                                .to_string()
                        });

                    // For scoped/chained calls like Foo::bar() or obj.method(),
                    // emit a TypeRef for the type prefix so the struct/class
                    // appears as a dependency, not just the method.
                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &func, refs);

                    // Turbofish: `foo::<T>()` or `Vec::<String>::new()` —
                    // the function node may be a `generic_function` containing
                    // type_arguments.  Walk those args for TypeRefs.
                    if func.kind() == "generic_function" {
                        if let Some(type_args) = func.child_by_field_name("type_arguments") {
                            super::symbols::extract_type_refs_from_type_node(
                                &type_args,
                                source,
                                source_symbol_index,
                                refs,
                            );
                        }
                    }

                    if !target_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: func.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                // Recurse into the entire call node (function + arguments) so that
                // nested calls in the callee chain and closure arguments are all found.
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, source, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, source, source_symbol_index, refs);
                }
            }

            "closure_expression" => {
                // Emit Variable symbols for closure parameters, then recurse into body.
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_closure_params(&child, source, source_symbol_index, syms);
                    extract_calls_from_body_with_symbols(&child, source, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, source, source_symbol_index, refs);
                }
            }

            // `type_identifier` encountered in expression contexts (match arms, closures, etc.)
            // Emit TypeRef for the type name.
            "type_identifier" => {
                let name = node_text(&child, source);
                if !name.is_empty() && !is_rust_primitive(&name) {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }

            // `scoped_type_identifier` like `std::io::Result` in match arms or type contexts
            "scoped_type_identifier" => {
                let name = node_text(&child, source);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }

            _ => {
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, source, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, source, source_symbol_index, refs);
                }
            }
        }
    }
}

/// Emit Variable symbols for each identifier in a `closure_parameters` node.
///
/// Handles:
/// - `|x|`             → identifier
/// - `|x: Type|`       → identifier with type annotation
/// - `|mut x|`         → mutable binding
/// - `|(a, b)|`        → destructured tuple pattern
fn extract_closure_params(
    closure_node: &Node,
    source: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = closure_node.walk();
    for child in closure_node.children(&mut cursor) {
        if child.kind() == "closure_parameters" {
            let mut pc = child.walk();
            for param in child.children(&mut pc) {
                match param.kind() {
                    "identifier" => {
                        let name = node_text(&param, source);
                        if !name.is_empty() && name != "|" {
                            symbols.push(make_closure_variable(name, &param, parent_index));
                        }
                    }
                    // `x: Type` — the identifier is a child named `pattern`
                    "parameter" => {
                        if let Some(pat) = param.child_by_field_name("pattern") {
                            let name = node_text(&pat, source);
                            if !name.is_empty() {
                                symbols.push(make_closure_variable(name, &pat, parent_index));
                            }
                        }
                    }
                    // `mut x`
                    "mut_specifier" | "mutable_specifier" => {}
                    _ => {}
                }
            }
        }
    }
}

fn make_closure_variable(name: String, node: &Node, parent_index: usize) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.clone(),
        qualified_name: name,
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: Some(parent_index),
    }
}

/// Build a structured member-access chain from a Rust call expression's function node.
///
/// Returns `None` for bare single-segment identifiers.
fn build_chain(node: Node, source: &str) -> Option<MemberChain> {
    if node.kind() == "identifier" || node.kind() == "self" {
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

        "self" => {
            segments.push(ChainSegment {
                name: "self".to_string(),
                node_kind: "self".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "field_expression" => {
            let value = node.child_by_field_name("value")?;
            let field = node.child_by_field_name("field")?;
            build_chain_inner(value, source, segments)?;
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

        "scoped_identifier" => {
            let text = node_text(&node, source);
            let parts: Vec<&str> = text.split("::").collect();
            if parts.len() < 2 {
                segments.push(ChainSegment {
                    name: text,
                    node_kind: "scoped_identifier".to_string(),
                    kind: SegmentKind::Identifier,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                });
            } else {
                for (i, part) in parts.iter().enumerate() {
                    let kind = if i == 0 {
                        SegmentKind::Identifier
                    } else {
                        SegmentKind::Property
                    };
                    segments.push(ChainSegment {
                        name: part.trim().to_string(),
                        node_kind: "scoped_identifier".to_string(),
                        kind,
                        declared_type: None,
                        type_args: vec![],
                        optional_chaining: false,
                    });
                }
            }
            Some(())
        }

        "call_expression" => {
            let func = node.child_by_field_name("function")?;
            build_chain_inner(func, source, segments)
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// extern crate import
// ---------------------------------------------------------------------------

/// Emit an `Imports` edge for `extern crate foo;`.
///
/// tree-sitter-rust shape:
/// ```text
/// extern_crate_declaration
///   "extern" "crate"
///   name: identifier  "foo"
///   ["as" alias: identifier]
/// ```
pub(super) fn extract_extern_crate(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, source);
    if name.is_empty() || name == "self" {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index: current_symbol_count,
        target_name: name,
        kind: EdgeKind::Imports,
        line: name_node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// Use declaration / import reference extraction
// ---------------------------------------------------------------------------

/// Walk a `use_declaration` node and emit `Import` references for every
/// leaf name that is actually imported.
pub(super) fn extract_use_names(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "scoped_identifier"
            | "scoped_use_list"
            | "use_as_clause"
            | "use_wildcard"
            | "identifier"
            | "use_list" => {
                walk_use_tree(&child, source, refs, current_symbol_count, "");
            }
            _ => {}
        }
    }
}

fn walk_use_tree(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
    prefix: &str,
) {
    match node.kind() {
        "scoped_identifier" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let path = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();

            if name.is_empty() {
                return;
            }

            let module = build_module_path(prefix, &path);
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module: if module.is_empty() { None } else { Some(module) },
                chain: None,
            });
        }

        "scoped_use_list" => {
            let path = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let new_prefix = build_module_path(prefix, &path);

            if let Some(list) = node.child_by_field_name("list") {
                walk_use_tree(&list, source, refs, current_symbol_count, &new_prefix);
            }
        }

        "use_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "{" | "}" | "," => {}
                    _ => walk_use_tree(&child, source, refs, current_symbol_count, prefix),
                }
            }
        }

        "use_as_clause" => {
            let alias = node
                .child_by_field_name("alias")
                .map(|n| node_text(&n, source));
            let original = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source));

            let target = alias.or(original).unwrap_or_default();
            if target.is_empty() {
                return;
            }

            let module = if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            };

            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: target,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module,
                chain: None,
            });
        }

        "use_wildcard" => {
            let module = if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            };
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: "*".to_string(),
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module,
                chain: None,
            });
        }

        "identifier" => {
            let name = node_text(node, source);
            if name.is_empty() {
                return;
            }
            let module = if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            };
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module,
                chain: None,
            });
        }

        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_use_tree(&child, source, refs, current_symbol_count, prefix);
            }
        }
    }
}

fn build_module_path(prefix: &str, path: &str) -> String {
    match (prefix.is_empty(), path.is_empty()) {
        (true, true) => String::new(),
        (true, false) => path.to_string(),
        (false, true) => prefix.to_string(),
        (false, false) => format!("{prefix}::{path}"),
    }
}

// ---------------------------------------------------------------------------
// Type name extraction helper (for type_cast_expression targets)
// ---------------------------------------------------------------------------


/// Extract a simple type name from a Rust type node, unwrapping references and
/// generic wrappers to their base name.
///
/// Handles:
/// - `type_identifier`          → `"Foo"`
/// - `scoped_type_identifier`   → last segment of `foo::Bar`
/// - `generic_type`             → base type name from `Vec<T>`
/// - `reference_type`           → recurse into inner type (`&T`, `&mut T`)
/// - `pointer_type` (raw ptr)   → recurse into inner type (`*const T`)
/// - `abstract_type`            → `impl Trait` → trait name
/// - `dynamic_trait_type`       → `dyn Error + Send` → first trait name
/// - `array_type`               → `[T; N]` → element type name
/// - `tuple_type`               → `(A, B)` → first non-primitive element
pub(super) fn rust_type_node_name(node: &Node, source: &str) -> String {
    match node.kind() {
        "type_identifier" => node_text(node, source),
        "scoped_type_identifier" => {
            // Last segment — `foo::Bar` → `"Bar"`.
            node.child_by_field_name("name")
                .map(|n| node_text(&n, source))
                .unwrap_or_else(|| {
                    let text = node_text(node, source);
                    text.rsplit("::").next().unwrap_or(&text).to_string()
                })
        }
        "generic_type" => {
            // `Vec<T>` — take the base type.
            node.child_by_field_name("type")
                .map(|n| rust_type_node_name(&n, source))
                .unwrap_or_default()
        }
        "reference_type" | "pointer_type" => {
            // `&T`, `&mut T`, `*const T` — unwrap to inner type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() && child.kind() != "mutable_specifier" {
                    let name = rust_type_node_name(&child, source);
                    if !name.is_empty() {
                        return name;
                    }
                }
            }
            String::new()
        }
        "abstract_type" => {
            // `impl Trait` — extract trait name (first named child after `impl` keyword).
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    let name = rust_type_node_name(&child, source);
                    if !name.is_empty() {
                        return name;
                    }
                }
            }
            String::new()
        }
        "dynamic_trait_type" => {
            // `dyn Error + Send` — use the first trait name (skip `dyn` keyword).
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    let name = rust_type_node_name(&child, source);
                    if !name.is_empty() {
                        return name;
                    }
                }
            }
            String::new()
        }
        "array_type" => {
            // `[T; N]` — element type is the `element` field.
            node.child_by_field_name("element")
                .map(|n| rust_type_node_name(&n, source))
                .unwrap_or_default()
        }
        "tuple_type" => {
            // `(A, B, C)` — return the first named element type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    let name = rust_type_node_name(&child, source);
                    if !name.is_empty() {
                        return name;
                    }
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

