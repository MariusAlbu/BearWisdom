// =============================================================================
// parser/extractors/rust/mod.rs  —  Rust symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Struct, Enum, EnumMember, Interface (trait), Method, Function,
//   TypeAlias, Variable (static), Namespace (mod), Test
//
// REFERENCES:
//   - `use` declarations     → Import edges (recursive use-tree walking)
//   - `call_expression`      → Calls edges
//
// Approach:
//   Single-pass recursive CST walk. No scope tree — qualified names are built
//   by threading a `qualified_prefix` string through the recursion. `impl`
//   blocks are not symbols themselves; they set the prefix for their methods.
// =============================================================================


use super::{calls, symbols, helpers, decorators, patterns};
use crate::types::ExtractionResult;
use crate::types::{ExtractedRef, ExtractedSymbol};
use tree_sitter::{Node, Parser};

// Re-exports required by rust_tests.rs (`use super::*`).
pub(crate) use crate::types::{EdgeKind, SymbolKind, Visibility};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------



/// Extract all symbols and references from Rust source code.
pub fn extract(source: &str) -> ExtractionResult {
    let language = tree_sitter_rust::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to set Rust grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return ExtractionResult {
                symbols: vec![],
                refs: vec![],
                routes: vec![],
                db_sets: vec![],
                has_errors: true,
                connection_points: Vec::new(),
                demand_contributions: Vec::new(),
                alias_targets: Vec::new(),
            }
        }
    };

    let mut syms = Vec::new();
    let mut refs = Vec::new();

    let root = tree.root_node();

    extract_from_node(
        root,
        source,
        &mut syms,
        &mut refs,
        None,
        "",
    );

    // Second pass: scan the full CST for type_identifier and scoped_type_identifier
    // nodes, emitting TypeRef for each non-primitive type found anywhere in the file.
    if !syms.is_empty() {
        scan_all_type_identifiers(root, source, 0, &mut refs);
    }

    // Third pass: enrich Calls refs that have a qualified chain (≥2 segments)
    // but no module set.  Build an import map from the Imports refs already
    // emitted — `target_name → module` — then for each qualifying Calls ref
    // whose first chain segment matches an imported name, copy that module onto
    // the ref.  This lets the resolver trace `DbPool::new()` back to
    // `crate::db` because `DbPool` was imported via `use crate::db::DbPool`.
    {
        let import_map: rustc_hash::FxHashMap<String, String> = refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .filter_map(|r| {
                r.module.as_ref().map(|m| (r.target_name.clone(), m.clone()))
            })
            .collect();

        for r in refs.iter_mut() {
            if r.kind == EdgeKind::Calls && r.module.is_none() {
                if let Some(chain) = &r.chain {
                    if chain.segments.len() >= 2 {
                        let first = &chain.segments[0].name;
                        if let Some(module) = import_map.get(first) {
                            r.module = Some(module.clone());
                        }
                    }
                }
            }
        }
    }

    // Fourth pass: drop refs whose `target_name` contains characters that
    // can never be part of a Rust identifier or path. The chain extractor
    // and call-expression fallback occasionally capture stray punctuation
    // from malformed CST nodes — `(config.normalize_type)(arg)` lands as
    // `normalize_type)`, `.collect::<Result<Vec<_>>>()` lands as
    // `Result<Vec<_>>>`, multi-line snippets land with embedded newlines.
    // None of these can ever resolve to a real symbol; keeping them just
    // pollutes the unresolved-refs table.
    refs.retain(|r| is_valid_rust_target_name(&r.target_name));

    let has_errors = tree.root_node().has_error();
    ExtractionResult::new(syms, refs, has_errors)
}

/// Whether `name` could be a Rust identifier or path. Allows alphanumerics,
/// underscores, `::` separators, leading `!` (none — Rust forbids), trailing
/// `!` (macro invocation in some emit paths), `*` (wildcards), `?` is NOT
/// allowed — neither are angle brackets, parens, braces, brackets, quotes,
/// commas, whitespace, semicolons, or backslashes. Returns true for the
/// empty string so callers that explicitly accept empty targets aren't
/// disturbed; the main extract path already filters empty.
fn is_valid_rust_target_name(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    for c in name.chars() {
        match c {
            '_' | ':' | '*' | '!' => continue,
            ch if ch.is_ascii_alphanumeric() => continue,
            // Emoji / non-ascii letters in identifiers are allowed in Rust;
            // gate on the broader unicode XID rule to keep CJK identifier
            // names out of the reject path.
            ch if ch.is_alphanumeric() => continue,
            _ => return false,
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn extract_from_node(
    node: Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" | "function_signature_item" => {
                if let Some(sym) =
                    symbols::extract_function(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let fn_qname = sym.qualified_name.clone();
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Emit TypeRefs for parameter types and return type.
                    symbols::extract_fn_signature_type_refs(&child, source, idx, refs);
                    // Emit Variable symbols for callable-typed parameters
                    // (`impl Fn(...)`, `&dyn Fn(...)`, `fn(...)` etc.) so
                    // calls in the body that look like `cb(arg)` can resolve
                    // to the parameter via scope-chain lookup. Skip the
                    // common case (non-callable parameters) — adding every
                    // parameter as a Variable would explode the symbol
                    // table without helping resolution.
                    symbols::extract_callable_fn_params(
                        &child, source, idx, &fn_qname, symbols,
                    );
                    // where-clause and type-parameter bounds → TypeRef edges.
                    // Iterate children by kind rather than field_name to avoid
                    // grammar-version sensitivity.
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
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls_from_body_with_symbols(&body, source, idx, refs, Some(symbols));
                    }
                }
            }

            "struct_item" | "union_item" => {
                // `union_item` has the same field layout as `struct_item` in
                // tree-sitter-rust; reuse the struct extractor and emit Struct kind.
                if let Some(sym) =
                    symbols::extract_struct(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let struct_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    let line = sym.start_line;
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Extract field symbols and TypeRefs for field types.
                    symbols::extract_struct_fields(&child, source, idx, &struct_prefix, symbols, refs);
                    // Synthesize method symbols for each #[derive(...)] trait.
                    decorators::synthesize_derive_methods(&child, source, idx, &struct_prefix, line, symbols);
                }
            }

            "enum_item" => {
                if let Some(sym) =
                    symbols::extract_enum(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    let line = sym.start_line;
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        symbols::extract_enum_variants(&body, source, Some(idx), &new_prefix, symbols, refs);
                    }
                    // Synthesize method symbols for each #[derive(...)] trait.
                    decorators::synthesize_derive_methods(&child, source, idx, &new_prefix, line, symbols);
                }
            }

            "trait_item" => {
                if let Some(sym) =
                    symbols::extract_trait(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Supertrait bounds: `trait Foo: Bar + Baz` -> Inherits edges.
                    patterns::extract_supertrait_bounds(&child, source, idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        // Extract associated types declared in the trait body.
                        symbols::extract_trait_associated_types(&body, source, idx, &new_prefix, symbols, refs);
                        extract_from_node(body, source, symbols, refs, Some(idx), &new_prefix);
                    }
                }
            }

            "impl_item" => {
                calls::extract_impl(&child, source, symbols, refs, qualified_prefix);
            }

            "type_item" => {
                if let Some(sym) =
                    symbols::extract_type_alias(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Emit TypeRef for the RHS type (covers `type_identifier` nodes in
                    // the type alias body — e.g. `type Foo = SomeType<Bar>`).
                    if let Some(type_node) = child.child_by_field_name("type") {
                        symbols::extract_type_refs_from_type_node(&type_node, source, idx, refs);
                    }
                }
            }

            "const_item" => {
                if let Some(sym) =
                    symbols::extract_const(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Emit TypeRef for the type annotation.
                    if let Some(type_node) = child.child_by_field_name("type") {
                        symbols::extract_type_refs_from_type_node(&type_node, source, idx, refs);
                    }
                }
            }

            "static_item" => {
                if let Some(sym) =
                    symbols::extract_static(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Emit TypeRef for the type annotation.
                    if let Some(type_node) = child.child_by_field_name("type") {
                        symbols::extract_type_refs_from_type_node(&type_node, source, idx, refs);
                    }
                }
            }

            "mod_item" => {
                if let Some(sym) =
                    symbols::extract_mod(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_from_node(body, source, symbols, refs, Some(idx), &new_prefix);
                    }
                }
            }

            "use_declaration" => {
                calls::extract_use_names(&child, source, refs, symbols.len());
            }

            // `extern "C" { fn malloc(size: usize) -> *mut u8; }`
            // Walk the declaration_list body and emit Function symbols for each
            // `foreign_item` function declaration.
            "foreign_mod_item" => {
                if let Some(body) = child.child_by_field_name("body") {
                    let mut bc = body.walk();
                    for decl in body.children(&mut bc) {
                        if decl.kind() == "function_item" || decl.kind() == "function_signature_item" {
                            if let Some(sym) =
                                symbols::extract_function(&decl, source, parent_index, qualified_prefix)
                            {
                                let idx = symbols.len();
                                symbols.push(sym);
                                decorators::extract_decorators(&decl, source, idx, refs);
                            }
                        }
                    }
                }
            }

            // `extern crate foo;` — emit an Imports edge for the crate name.
            "extern_crate_declaration" => {
                calls::extract_extern_crate(&child, source, refs, symbols.len());
            }

            // `macro_rules! foo { ... }` — tree-sitter-rust 0.24 emits `macro_definition`.
            // Emit a Function symbol for the macro name.
            "macro_definition" => {
                if let Some(sym) =
                    symbols::extract_macro_rules(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                }
            }

            // Module-level macro invocations: `lazy_static! { ... }`, `global_allocator!(...)`.
            // Emit a Calls edge for the macro name (same as body-level macros).
            "macro_invocation" => {
                let source_idx = parent_index.unwrap_or(0);
                // Emit Calls ref for the macro name itself.
                if let Some(macro_node) = child.child_by_field_name("macro") {
                    let raw = helpers::node_text(&macro_node, source);
                    let raw = raw.trim_end_matches('!');
                    // Split `prefix::name!` into module + name so the
                    // resolver can route to the owning crate. Mirrors the
                    // body walker's macro_invocation arm in calls.rs.
                    let (module, target) = match raw.rsplit_once("::") {
                        Some((prefix, leaf)) if !prefix.is_empty() && !leaf.is_empty() => {
                            (Some(prefix.to_string()), leaf.to_string())
                        }
                        _ => (None, raw.to_string()),
                    };
                    if !target.is_empty() {
                        refs.push(crate::types::ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: target,
                            kind: crate::types::EdgeKind::Calls,
                            line: macro_node.start_position().row as u32,
                            module,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
                                                    call_args: Vec::new(),
});
                    }
                }
                // Recurse into token-tree arguments for nested calls.
                calls::extract_calls_from_body_with_symbols(
                    &child,
                    source,
                    source_idx,
                    refs,
                    Some(symbols),
                );
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_from_node(
                    child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Full-tree type_identifier scan
// ---------------------------------------------------------------------------

/// Recursively scan the entire CST and emit a TypeRef for every `type_identifier`
/// or `scoped_type_identifier` node that is in a type-annotation position.
///
/// Nodes are skipped when they appear inside:
///   - The `value` field of a `let_declaration` — the RHS of a let binding is
///     an expression, not a type annotation.  `let x = Foo::new()` should not
///     emit TypeRef for `Foo` from this pass (calls.rs handles that separately).
///   - `attribute_item` subtrees — attributes are macro invocations whose names
///     and arguments are not symbol references (`#[derive(Debug)]`, `#[serde(...)]`).
fn scan_all_type_identifiers(
    node: tree_sitter::Node,
    source: &str,
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Skip the value (RHS) of let declarations — type_identifier nodes there
        // are constructor/function names in expressions, not type annotations.
        if child.kind() == "let_declaration" {
            if let Some(value_node) = child.child_by_field_name("value") {
                // Only scan the type annotation subtree, not the value subtree.
                // The `type` field holds the explicit `: T` annotation.
                if let Some(type_node) = child.child_by_field_name("type") {
                    scan_all_type_identifiers(type_node, source, sym_idx, refs);
                }
                // Recurse into everything that is NOT the value subtree.
                let value_id = value_node.id();
                let mut lc = child.walk();
                for lc_child in child.children(&mut lc) {
                    if lc_child.id() == value_id {
                        continue;
                    }
                    // Also skip the type node — already handled above.
                    if child.child_by_field_name("type").map(|n| n.id()) == Some(lc_child.id()) {
                        continue;
                    }
                    scan_all_type_identifiers(lc_child, source, sym_idx, refs);
                }
                continue;
            }
            // No value field — scan all children normally.
        }

        // Skip attribute_item nodes entirely — their contents are macro invocations,
        // not type references.  `extract_decorators` handles them in the main pass
        // for structured attributes on top-level items.
        if child.kind() == "attribute_item" {
            continue;
        }

        match child.kind() {
            "type_identifier" if child.is_named() => {
                let name = helpers::node_text(&child, source);
                if !name.is_empty() && !symbols::is_rust_primitive(&name) {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
            "scoped_type_identifier" if child.is_named() => {
                // `foo::Bar` — extract the leaf name (last segment).
                let name = child
                    .child_by_field_name("name")
                    .map(|n| helpers::node_text(&n, source))
                    .unwrap_or_else(|| {
                        let text = helpers::node_text(&child, source);
                        text.rsplit("::").next().unwrap_or(&text).to_string()
                    });
                if !name.is_empty() && !symbols::is_rust_primitive(&name) {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
                // Don't recurse into scoped_type_identifier children — we already extracted the leaf.
                continue;
            }
            _ => {}
        }
        scan_all_type_identifiers(child, source, sym_idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

