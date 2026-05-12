// =============================================================================
// parser/extractors/ruby/mod.rs  —  Ruby symbol and reference extractor
// =============================================================================


use super::{symbols};
use super::symbols::{
    extract_call_statement, extract_class, extract_method, extract_module,
    extract_singleton_class, extract_singleton_method,
};

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{ExtractedRef, ExtractedSymbol};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

pub(crate) static RUBY_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class",            name_field: "name" },
    ScopeKind { node_kind: "module",           name_field: "name" },
    ScopeKind { node_kind: "method",           name_field: "name" },
    ScopeKind { node_kind: "singleton_method", name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_ruby::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Ruby grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let src = source.as_bytes();
    let root = tree.root_node();

    // Build scope tree for qualified-name lookups (used in scope_path).
    let _scope_tree = scope_tree::build(root, src, RUBY_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_from_node(root, src, &mut symbols, &mut refs, None, "", false);

    // Second pass: scan the full CST for `constant` and `scope_resolution` nodes,
    // emitting TypeRef for each one found anywhere in the file (including inside
    // method bodies that the main walker does not descend into directly).
    if !symbols.is_empty() {
        scan_all_constants(root, src, 0, &mut refs);
    }

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

pub(super) fn extract_from_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class" => {
                extract_class(&child, src, symbols, refs, parent_index, qualified_prefix);
            }

            "module" => {
                extract_module(&child, src, symbols, refs, parent_index, qualified_prefix);
            }

            "method" => {
                extract_method(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                );
            }

            "singleton_method" => {
                extract_singleton_method(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            // `class << self ... end` — singleton class / eigenclass.
            "singleton_class" => {
                extract_singleton_class(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "call" => {
                extract_call_statement(
                    &child,
                    src,
                    refs,
                    symbols.len(),
                    parent_index,
                    inside_class,
                    symbols,
                );
                // Also recurse into the call's children so nested calls in
                // arguments (e.g. `foo(bar(baz()))`) at class/module body level
                // are captured.  extract_call_statement only handles one level.
                let sym_idx = parent_index.unwrap_or(0);
                super::calls::extract_calls_from_body(
                    &child,
                    src,
                    sym_idx,
                    refs,
                );
            }

            // `method_call` is an alternative grammar node for method calls (tree-sitter-ruby
            // may parse some calls as `method_call` instead of `call`). Extract calls from it.
            "method_call" => {
                let sym_idx = parent_index.unwrap_or(0);
                super::calls::extract_calls_from_body(
                    &child,
                    src,
                    sym_idx,
                    refs,
                );
            }

            // `Foo::Bar` — scope resolution used as a type reference (e.g. in
            // assignments, conditionals, or as standalone constant refs).
            "scope_resolution" => {
                let sym_idx = parent_index.unwrap_or(0);
                let type_name = super::helpers::node_text(&child, src);
                if !type_name.is_empty() {
                    refs.push(crate::types::ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: type_name,
                        kind: crate::types::EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }

            // Bare constant reference (e.g. `TIMEOUT`, `MyClass`) — emit TypeRef
            // when encountered outside a method body (inside method bodies the
            // `extract_calls_from_body` path handles them via the `call` arm).
            "constant" => {
                let sym_idx = parent_index.unwrap_or(0);
                let type_name = super::helpers::node_text(&child, src);
                if !type_name.is_empty() {
                    refs.push(crate::types::ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: type_name,
                        kind: crate::types::EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }

            // `include Foo` / `extend Bar` / `prepend Mixin` — bare command at
            // module or class body level.  tree-sitter-ruby parses these as
            // `call` in most contexts, but `command` nodes appear for no-paren
            // method calls that are not syntactically calls.
            "command" => {
                super::calls::extract_calls_from_body(
                    &child,
                    src,
                    parent_index.unwrap_or(0),
                    refs,
                );
            }

            "command_call" => {
                super::calls::extract_calls_from_body(
                    &child,
                    src,
                    parent_index.unwrap_or(0),
                    refs,
                );
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_from_node(
                    child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Full-tree constant scan
// ---------------------------------------------------------------------------

/// Recursively scan the entire CST and emit a TypeRef for every `constant` or
/// `scope_resolution` node found anywhere in the file.
///
/// Ruby has no static type system, so all constants (PascalCase names like
/// `User`, `ActiveRecord::Base`) are user-defined classes or modules.
fn scan_all_constants(
    node: tree_sitter::Node,
    src: &[u8],
    sym_idx: usize,
    refs: &mut Vec<crate::types::ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "constant" if child.is_named() => {
                let name = super::helpers::node_text(&child, src);
                if !name.is_empty() {
                    refs.push(crate::types::ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: crate::types::EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
            "scope_resolution" if child.is_named() => {
                // `Foo::Bar` — extract the rightmost constant segment.
                let full = super::helpers::node_text(&child, src);
                let name = full.rsplit("::").next().unwrap_or(&full).to_string();
                if !name.is_empty() {
                    refs.push(crate::types::ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: crate::types::EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
                // Don't recurse into scope_resolution — we already extracted the name.
                continue;
            }
            _ => {}
        }
        scan_all_constants(child, src, sym_idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

