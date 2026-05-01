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
        // Carry the impl block's own type_parameters into the signature
        // so the engine's generic_params parser picks them up. Without
        // this, refs to `T` inside an `impl<T> Foo for T` block end up
        // as unresolved type references.
        let signature = match node.child_by_field_name("type_parameters") {
            Some(tp) => format!("impl{} {type_name}", node_text(&tp, source)),
            None => format!("impl {type_name}"),
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
            signature: Some(signature),
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
                byte_offset: 0,
                            namespace_segments: Vec::new(),
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
                byte_offset: 0,
                            namespace_segments: Vec::new(),
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
                    // Extract attributes on the method (e.g. #[test], #[tokio::test],
                    // #[instrument]) — these are `attribute_item` previous siblings.
                    super::decorators::extract_decorators(&child, source, idx, refs);
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
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
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

            // Function-scoped `use` statements: `use std::sync::OnceLock;`
            // declared inside a function body. The top-level extractor only
            // handles `use_declaration` at module scope, but Rust permits
            // `use` anywhere a statement is valid — and idiomatic code uses
            // function-local imports for trait extension methods, lazy
            // statics, and one-off helpers. Without this, any reference to
            // an imported name inside the function body sees no import
            // entry and falls through to unresolved.
            "use_declaration" => {
                extract_use_names(&child, source, refs, source_symbol_index);
                // No further descent — `extract_use_names` handles the
                // entire subtree.
            }

            // Function-scoped `macro_rules! foo { … }`. Same rationale as
            // `use_declaration` above: top-level extract.rs handles the
            // module-scope case, but local macros inside builders / closures
            // (e.g. the `reg_eco!` helper in `default_registry()`) are
            // declared inside `OnceLock::get_or_init(|| { … })`. Without
            // an extraction arm here, the macro symbol never lands in the
            // index and every same-file invocation lands as an unresolved
            // call ref.
            "macro_definition" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    if let Some(sym) = super::symbols::extract_macro_rules(
                        &child, source, None, "",
                    ) {
                        syms.push(sym);
                    }
                }
            }

            // Function-scoped statics + consts: `static LOCATOR: OnceLock<…>
            // = OnceLock::new();` and `const FOO: usize = 42;` declared
            // inside a function body. The OnceLock-based shared_locator
            // pattern across the ecosystem crate keeps such statics inside
            // the helper function — same scoping issue as `use_declaration`
            // and `macro_definition`. Without local extraction the
            // subsequent `LOCATOR.get_or_init(…)` reference can't resolve.
            "static_item" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    if let Some(sym) = super::symbols::extract_static(
                        &child, source, None, "",
                    ) {
                        syms.push(sym);
                    }
                }
            }
            "const_item" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    if let Some(sym) = super::symbols::extract_const(
                        &child, source, None, "",
                    ) {
                        syms.push(sym);
                    }
                }
            }

            // Function-scoped type definitions: `struct Foo { … }`,
            // `enum Bar { … }`, `type Alias = …;` declared inside a
            // function body. Same scoping family as PR 99/100 — the
            // module-level extractor never sees these because they live
            // beneath a `function_item`, so any subsequent reference
            // (constructor calls, type annotations, pattern matches)
            // misses the symbol and lands as unresolved. Real examples in
            // this codebase: `CoordResult` in `ecosystem/nuget.rs::nuget_coord_artifacts`,
            // `HandlerMatch` in `languages/csharp/connectors.rs`. Together
            // these account for ~25 unresolved refs on the BW self-index.
            // No body recursion needed — type definitions don't contain
            // call sites the walker cares about (field/variant types are
            // emitted via the symbols extractor).
            "struct_item" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    if let Some(sym) = super::symbols::extract_struct(
                        &child, source, None, "",
                    ) {
                        syms.push(sym);
                    }
                }
            }
            "enum_item" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    if let Some(sym) = super::symbols::extract_enum(
                        &child, source, None, "",
                    ) {
                        syms.push(sym);
                    }
                }
            }
            "type_item" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    if let Some(sym) = super::symbols::extract_type_alias(
                        &child, source, None, "",
                    ) {
                        syms.push(sym);
                    }
                }
            }

            // `println!()`, `vec![]`, `format!()`, custom macros.
            // Can't expand them, but we emit a Calls edge for the macro name.
            "macro_invocation" => {
                if let Some(macro_node) = child.child_by_field_name("macro") {
                    let raw = node_text(&macro_node, source);
                    // Strip trailing `!` if present (some grammars include it).
                    let raw = raw.trim_end_matches('!');
                    // Split scoped macro paths so the resolver can route
                    // `rusqlite::params!` to the `rusqlite` crate by module
                    // instead of looking for a flat `rusqlite::params`
                    // symbol that never exists. Mirrors `scoped_type_identifier`
                    // handling — `prefix::leaf` → module=prefix, name=leaf.
                    let (module, target) = match raw.rsplit_once("::") {
                        Some((prefix, leaf)) if !prefix.is_empty() && !leaf.is_empty() => {
                            (Some(prefix.to_string()), leaf.to_string())
                        }
                        _ => (None, raw.to_string()),
                    };
                    if !target.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: target,
                            kind: EdgeKind::Calls,
                            line: macro_node.start_position().row as u32,
                            module,
                            chain: None,
                            byte_offset: macro_node.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
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
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
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
                if let Some(syms) = symbols.as_deref_mut() {
                    let syms_before = syms.len();

                    // Emit TypeRef for the declared type: `let x: MyType = ...`
                    // Attach to the enclosing function (source_symbol_index) since
                    // the Variable symbols haven't been pushed yet.
                    if let Some(type_node) = child.child_by_field_name("type") {
                        super::symbols::extract_type_refs_from_type_node(
                            &type_node,
                            source,
                            source_symbol_index,
                            refs,
                        );
                    }

                    // Reuse the pattern extractor — handles identifiers, tuple patterns, etc.
                    // `let_declaration` and `let_condition` share the same `pattern` field.
                    super::patterns::extract_let_condition_pattern(
                        &child,
                        source,
                        source_symbol_index,
                        syms,
                        refs,
                    );

                    // For single-binding `let x = <rhs>` without an explicit type
                    // annotation, infer the variable's type from the RHS expression.
                    // Only handles the simple case (one new Variable symbol pushed).
                    let has_explicit_type = child.child_by_field_name("type").is_some();
                    if !has_explicit_type && syms.len() == syms_before + 1 {
                        let var_sym_idx = syms_before;
                        if let Some(value_node) = child.child_by_field_name("value") {
                            infer_rust_variable_type(value_node, source, var_sym_idx, refs);
                        }
                    }

                    extract_calls_from_body_with_symbols(&child, source, source_symbol_index, refs, Some(syms));
                } else {
                    // No symbol tracking — just handle explicit type refs and recurse.
                    if let Some(type_node) = child.child_by_field_name("type") {
                        super::symbols::extract_type_refs_from_type_node(
                            &type_node,
                            source,
                            source_symbol_index,
                            refs,
                        );
                    }
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
                            byte_offset: name_node.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
});
                        // Also emit TypeRef so the type graph is connected.
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::TypeRef,
                            line: name_node.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
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
                            byte_offset: func.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
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
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }

            // `scoped_type_identifier` like `std::io::Result` in match arms or type contexts
            "scoped_type_identifier" => {
                let full_name = node_text(&child, source);
                if !full_name.is_empty() {
                    // Split `prefix::leaf` so the resolver sees the prefix in
                    // `module` and the leaf in `target_name`. Crucial for
                    // `Self::Variant` patterns (`match self { Self::Foo => … }`)
                    // — without splitting, the resolver sees a target of
                    // `Self::Foo` and can't route to the enclosing type's
                    // Foo member.
                    let (module, target) = match full_name.rsplit_once("::") {
                        Some((prefix, leaf)) if !prefix.is_empty() && !leaf.is_empty() => {
                            (Some(prefix.to_string()), leaf.to_string())
                        }
                        _ => (None, full_name),
                    };
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: target,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module,
                        chain: None,
                        byte_offset: 0,
                        namespace_segments: Vec::new(),
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
                    let trimmed = part.trim();
                    // First segment "Self" is the receiver type — must be
                    // tagged as SelfRef so the chain walker resolves it via
                    // the enclosing impl/struct/enum/trait. Without this,
                    // `Self::Variant` and `Self::method()` chains fall into
                    // the Identifier branch, which tries to look up "Self"
                    // as a regular type and always fails.
                    let kind = if i == 0 && trimmed == "Self" {
                        SegmentKind::SelfRef
                    } else if i == 0 {
                        SegmentKind::Identifier
                    } else {
                        SegmentKind::Property
                    };
                    segments.push(ChainSegment {
                        name: trimmed.to_string(),
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
        byte_offset: 0,
            namespace_segments: Vec::new(),
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
                byte_offset: 0,
                            namespace_segments: Vec::new(),
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

            // `target_name` is the alias when present, otherwise the original name.
            let target = alias
                .clone()
                .or_else(|| original.clone())
                .unwrap_or_default();
            if target.is_empty() {
                return;
            }

            // For `use foo::bar as fb` at the top level (prefix=""), derive the
            // module from the original path: "foo::bar" → module="foo", name="bar".
            // When the alias IS the original (no `as` clause reached this arm), fall
            // back to prefix as before.
            let module = if alias.is_some() {
                // Aliased import: module = parent of the original full path.
                let orig = original.as_deref().unwrap_or("");
                let full = build_module_path(prefix, orig);
                let parent = full.rsplit_once("::").map(|(p, _)| p.to_string());
                parent
            } else if prefix.is_empty() {
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
                byte_offset: 0,
                            namespace_segments: Vec::new(),
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
                byte_offset: 0,
                            namespace_segments: Vec::new(),
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
                byte_offset: 0,
                            namespace_segments: Vec::new(),
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

// ---------------------------------------------------------------------------
// Local variable type inference from RHS
// ---------------------------------------------------------------------------

/// Inspect the RHS expression of a `let` binding and emit a TypeRef for the
/// Variable symbol at `var_sym_idx` when the type can be determined:
///
/// - `let pool = DbPool::new(config)` → `call_expression` whose callee is a
///   `scoped_identifier` with uppercase prefix → TypeRef "DbPool"
/// - `let pool = DbPool::default()` → same pattern
/// - `let s = Foo { field: val }` → `struct_expression` → TypeRef "Foo"
/// - `let s = Foo(a, b)` → `call_expression` with uppercase `identifier` callee
///   → TypeRef "Foo" (tuple struct constructor)
///
/// Plain method calls (`let x = foo.bar()`) are already handled by the
/// chain-bearing TypeRef path in the engine's variable-inference pass and
/// do not need to be duplicated here.
fn infer_rust_variable_type(
    value_node: tree_sitter::Node,
    source: &str,
    var_sym_idx: usize,
    refs: &mut Vec<crate::types::ExtractedRef>,
) {
    match value_node.kind() {
        // `Foo { field: val }` — struct literal
        "struct_expression" => {
            if let Some(name_node) = value_node.child_by_field_name("name") {
                let type_name = rust_type_node_name(&name_node, source);
                if !type_name.is_empty() && !is_rust_primitive(&type_name) {
                    refs.push(crate::types::ExtractedRef {
                        source_symbol_index: var_sym_idx,
                        target_name: type_name,
                        kind: EdgeKind::TypeRef,
                        line: name_node.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }
        }

        // `Foo::new(...)`, `Foo::default()`, `Foo(a, b)` — call expression
        "call_expression" => {
            if let Some(func) = value_node.child_by_field_name("function") {
                let type_name = match func.kind() {
                    // `Foo::new(...)` — scoped_identifier whose path is an uppercase name
                    "scoped_identifier" => {
                        let path = func
                            .child_by_field_name("path")
                            .map(|n| node_text(&n, source))
                            .unwrap_or_default();
                        // Only treat as constructor if the path segment starts uppercase.
                        if path.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                            // Take just the type name (first segment before `::`)
                            path.split("::").next().unwrap_or(&path).to_string()
                        } else {
                            String::new()
                        }
                    }
                    // `Foo(a, b)` — bare uppercase identifier (tuple struct constructor)
                    "identifier" => {
                        let name = node_text(&func, source);
                        if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                            name
                        } else {
                            String::new()
                        }
                    }
                    _ => String::new(),
                };
                if !type_name.is_empty() && !is_rust_primitive(&type_name) {
                    refs.push(crate::types::ExtractedRef {
                        source_symbol_index: var_sym_idx,
                        target_name: type_name,
                        kind: EdgeKind::TypeRef,
                        line: func.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }
        }

        // `await expr` — unwrap and recurse once
        "await_expression" => {
            if let Some(inner) = value_node.child_by_field_name("value")
                .or_else(|| value_node.named_child(0))
            {
                infer_rust_variable_type(inner, source, var_sym_idx, refs);
            }
        }

        _ => {}
    }
}
