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

    // Third pass: scan ALL attribute_item nodes and emit TypeRef for any that
    // weren't already captured by extract_decorators (e.g., attributes on items
    // inside function bodies, #[cfg]-gated items, or non-standard item kinds).
    // Build an (line, name) dedup set from existing TypeRef refs to avoid doubles.
    {
        let existing: rustc_hash::FxHashSet<(u32, String)> = refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| (r.line, r.target_name.clone()))
            .collect();
        let mut new_refs: Vec<ExtractedRef> = Vec::new();
        scan_all_attribute_items(root, source, 0, &existing, &mut new_refs);
        refs.extend(new_refs);
    }

    let has_errors = tree.root_node().has_error();
    ExtractionResult::new(syms, refs, has_errors)
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
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Emit TypeRefs for parameter types and return type.
                    symbols::extract_fn_signature_type_refs(&child, source, idx, refs);
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
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Extract field symbols and TypeRefs for field types.
                    symbols::extract_struct_fields(&child, source, idx, &struct_prefix, symbols, refs);
                }
            }

            "enum_item" => {
                if let Some(sym) =
                    symbols::extract_enum(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        symbols::extract_enum_variants(&body, source, Some(idx), &new_prefix, symbols, refs);
                    }
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
/// or `scoped_type_identifier` node that is not a Rust primitive.
fn scan_all_type_identifiers(
    node: tree_sitter::Node,
    source: &str,
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
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
// Full-tree attribute_item scan
// ---------------------------------------------------------------------------

/// Recursively scan ALL `attribute_item` nodes and emit TypeRef for any that
/// weren't already emitted by the main pass (to avoid duplicates).
///
/// This catches attributes on items that `extract_from_node` falls through
/// (e.g., items inside function bodies, non-standard item kinds, closure params).
fn scan_all_attribute_items(
    node: tree_sitter::Node,
    source: &str,
    sym_idx: usize,
    existing: &rustc_hash::FxHashSet<(u32, String)>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "attribute_item" && child.is_named() {
            let line = child.start_position().row as u32;
            // Parse the attribute name from the attribute_item node.
            // Shape: attribute_item → attribute → identifier | scoped_identifier
            let name = parse_attr_name(&child, source);
            if !name.is_empty() && !existing.contains(&(line, name.clone())) {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line,
                    module: None,
                    chain: None,
                });
            }
            // Don't recurse into attribute_item children.
            continue;
        }
        scan_all_attribute_items(child, source, sym_idx, existing, refs);
    }
}

fn parse_attr_name(attr_item: &tree_sitter::Node, source: &str) -> String {
    let mut cursor = attr_item.walk();
    for child in attr_item.children(&mut cursor) {
        if child.kind() == "attribute" {
            let mut ac = child.walk();
            for attr_child in child.children(&mut ac) {
                match attr_child.kind() {
                    "identifier" => return helpers::node_text(&attr_child, source),
                    "scoped_identifier" => {
                        let full = helpers::node_text(&attr_child, source);
                        return full.rsplit("::").next().unwrap_or(&full).to_string();
                    }
                    _ => {}
                }
            }
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

