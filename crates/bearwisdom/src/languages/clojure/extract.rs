// =============================================================================
// languages/clojure/extract.rs — Clojure extractor (tree-sitter-based)
//
// In Clojure's tree-sitter grammar everything is a `list_lit`.
// We match the first `sym_lit` child to classify the form:
//
// SYMBOLS:
//   Function  — `defn`, `defn-`, `defmacro`, `defmulti`
//   Variable  — `def`, `defonce`
//   Struct    — `defrecord`, `deftype`
//   Interface — `defprotocol`, `definterface`
//   Namespace — `ns`
//
// REFERENCES:
//   Imports   — `ns` with `:require` / `:use` / `:import` vectors
//   Calls     — any `list_lit` whose head is a `sym_lit` referencing a known fn
// =============================================================================

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_clojure::LANGUAGE.into())
        .is_err()
    {
        return ExtractionResult::empty();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::empty(),
    };

    let src = source.as_bytes();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    walk_node(tree.root_node(), src, &mut symbols, &mut refs, None);

    ExtractionResult::new(symbols, refs, tree.root_node().has_error())
}

fn walk_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    if node.kind() == "list_lit" {
        if let Some(sym_idx_result) = process_list(node, src, symbols, refs, parent_idx) {
            // Already walked children inside process_list for def forms
            let _ = sym_idx_result;
            return;
        }
    }
    // Walk all children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, src, symbols, refs, parent_idx);
    }
}

/// Process a `list_lit` node. Returns Some(idx) if this is a def form.
fn process_list(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) -> Option<usize> {
    let values = list_values(node, src);
    if values.is_empty() {
        return None;
    }
    let head = &values[0];

    match head.as_str() {
        "defn" | "defn-" | "defmacro" | "defmulti" | "defmethod" => {
            let name = values.get(1).cloned().unwrap_or_default();
            if name.is_empty() { return None; }
            let vis = if head == "defn-" { Visibility::Private } else { Visibility::Public };
            let idx = push_sym(node, name, SymbolKind::Function, vis, symbols, parent_idx);
            walk_list_children(node, src, symbols, refs, Some(idx));
            Some(idx)
        }
        "def" | "defonce" => {
            let name = values.get(1).cloned().unwrap_or_default();
            if name.is_empty() { return None; }
            let idx = push_sym(node, name, SymbolKind::Variable, Visibility::Public, symbols, parent_idx);
            walk_list_children(node, src, symbols, refs, Some(idx));
            Some(idx)
        }
        "defrecord" | "deftype" => {
            let name = values.get(1).cloned().unwrap_or_default();
            if name.is_empty() { return None; }
            let idx = push_sym(node, name, SymbolKind::Struct, Visibility::Public, symbols, parent_idx);
            walk_list_children(node, src, symbols, refs, Some(idx));
            Some(idx)
        }
        "defprotocol" | "definterface" => {
            let name = values.get(1).cloned().unwrap_or_default();
            if name.is_empty() { return None; }
            let idx = push_sym(node, name, SymbolKind::Interface, Visibility::Public, symbols, parent_idx);
            walk_list_children(node, src, symbols, refs, Some(idx));
            Some(idx)
        }
        "ns" => {
            let ns_name = values.get(1).cloned().unwrap_or_default();
            if !ns_name.is_empty() {
                let idx = push_sym(node, ns_name, SymbolKind::Namespace, Visibility::Public, symbols, parent_idx);
                // Extract :require / :use / :import references
                extract_ns_refs(node, src, refs, idx);
                return Some(idx);
            }
            None
        }
        _ => {
            // Not a declaration — check if this looks like a function call
            // (head is a sym_lit that isn't a special form keyword)
            if !head.is_empty() && !head.starts_with(':') && !head.starts_with('"') {
                let sym_idx = parent_idx.unwrap_or(0);
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: head.clone(),
                    kind: EdgeKind::Calls,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
            None
        }
    }
}

/// Get the text of the first-level `sym_lit` / `kwd_lit` / `str_lit` children
/// of a `list_lit` (the "values" in Clojure parlance).
fn list_values(node: Node, src: &[u8]) -> Vec<String> {
    let mut vals = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "sym_lit" | "kwd_lit" | "str_lit" | "num_lit" => {
                let t = child.utf8_text(src).unwrap_or("").to_string();
                vals.push(t);
            }
            _ => {}
        }
    }
    vals
}

fn walk_list_children(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    let mut cursor = node.walk();
    let mut skip = 2usize; // skip head (`defn`) and name
    for child in node.children(&mut cursor) {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        walk_node(child, src, symbols, refs, parent_idx);
    }
}

fn extract_ns_refs(
    node: Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    sym_idx: usize,
) {
    // Walk children of the ns form looking for vec_lit children that start
    // with :require, :use, or :import
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "vec_lit" || child.kind() == "list_lit" {
            // First element should be a keyword like :require
            let mut inner = child.walk();
            let mut first = true;
            let mut is_import = false;
            for inner_child in child.children(&mut inner) {
                if inner_child.kind() == "kwd_lit" && first {
                    let kw = inner_child.utf8_text(src).unwrap_or("");
                    is_import = matches!(kw, ":require" | ":use" | ":import");
                    first = false;
                    continue;
                }
                if is_import {
                    // Each sub-form is a namespace reference
                    let name = extract_first_sym(inner_child, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index: sym_idx,
                            target_name: name,
                            kind: EdgeKind::Imports,
                            line: inner_child.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
            }
        }
    }
}

fn extract_first_sym(node: Node, src: &[u8]) -> String {
    if node.kind() == "sym_lit" {
        return node.utf8_text(src).unwrap_or("").to_string();
    }
    // For vec_lit like [some.ns :as alias], take the first sym_lit
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "sym_lit" {
            return child.utf8_text(src).unwrap_or("").to_string();
        }
    }
    String::new()
}

fn push_sym(
    node: Node,
    name: String,
    kind: SymbolKind,
    vis: Visibility,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) -> usize {
    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        qualified_name: name.clone(),
        name,
        kind,
        visibility: Some(vis),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: parent_idx,
    });
    idx
}
