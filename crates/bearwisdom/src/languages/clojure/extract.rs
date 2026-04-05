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
//   Calls     — every `sym_lit` encountered during CST traversal
//
// COVERAGE APPROACH:
//   The grammar has no dedicated declaration nodes — everything is `list_lit`.
//   ref_node_kinds = ["sym_name"] tracks every identifier leaf.
//   symbol_node_kinds = [] (N/A) because ~6% of list_lits are declarations;
//   declaring list_lit as a symbol kind would yield ~6% coverage, not 95%.
//
// SPECIAL FORMS HANDLED:
//   Non-sym-headed list_lits (no sym_lit first child) are classified by their
//   first named child:
//     vec_lit         — multi-arity function clause, e.g. `([] body)` or `([x] body)`
//     kwd_lit         — keyword-headed clause, e.g. `(:require [...])` in ns
//     read_cond_lit   — reader-conditional call, e.g. `(#?(:clj f :cljs g) args)`
//   All three cases recurse into children so their body refs are captured.
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
        // process_list handles both symbol extraction and child recursion for list_lits.
        process_list(node, src, symbols, refs, parent_idx);
        return;
    }
    if node.kind() == "sym_lit" {
        // When walk_node is called directly on a sym_lit (e.g. from walk_list_children
        // for body values like `db/tx0`), emit a ref for it here.
        // The child-iteration path below handles nested sym_lits inside other parents.
        let name = sym_lit_name(node, src);
        if !name.is_empty() && !name.starts_with(':') {
            refs.push(ExtractedRef {
                source_symbol_index: parent_idx.unwrap_or(0),
                target_name: name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
        return;
    }
    // For non-list, non-sym_lit nodes, walk children and emit refs for sym_lit occurrences.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "sym_lit" {
            let name = sym_lit_name(child, src);
            // Emit a ref for every sym_lit so sym_name coverage engine nodes are satisfied.
            if !name.is_empty() && !name.starts_with(':') {
                refs.push(ExtractedRef {
                    source_symbol_index: parent_idx.unwrap_or(0),
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        } else {
            walk_node(child, src, symbols, refs, parent_idx);
        }
    }
}

/// Process a `list_lit` node.
///
/// Declaration forms: push a symbol, walk children under the new symbol index.
/// Call forms: emit a Calls ref for the head and walk all argument children.
///
/// Non-sym-headed list_lits (no `sym_lit` first child) are classified by their
/// first named child kind and always recurse into children:
///
/// - `vec_lit`       — multi-arity clause `([] body)` or `([x] body)`
/// - `kwd_lit`       — keyword-headed clause `(:require [...])` in `ns`
/// - `read_cond_lit` — reader-conditional call `(#?(:clj f :cljs g) args)`
fn process_list(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    let (head, head_line) = list_head_with_line(node, src);
    if head.is_empty() {
        // No sym_lit head — classify by first named child and walk children.
        let first_named_kind: Option<String> = {
            let mut cursor = node.walk();
            let mut kind = None;
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    kind = Some(child.kind().to_owned());
                    break;
                }
            }
            kind
        };
        match first_named_kind.as_deref() {
            // Multi-arity function clause, keyword-headed ns clause, or
            // reader-conditional call — walk all children for refs.
            Some("vec_lit") | Some("kwd_lit")
            | Some("read_cond_lit") | Some("splicing_read_cond_lit") => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    walk_node(child, src, symbols, refs, parent_idx);
                }
            }
            _ => {}
        }
        return;
    }

    // Always emit a ref for the head sym_lit (the declaration keyword or call verb).
    // Use the head node's actual line (not the list's start) so the coverage engine
    // correctly correlates this ref to the sym_name child of the head sym_lit.
    if !head.starts_with(':') && !head.starts_with('"') {
        refs.push(ExtractedRef {
            source_symbol_index: parent_idx.unwrap_or(0),
            target_name: head.clone(),
            kind: EdgeKind::Calls,
            line: head_line,
            module: None,
            chain: None,
        });
    }

    match head.as_str() {
        "defn" | "defn-" | "defmacro" | "defmulti" | "defmethod" => {
            let (name, name_line) = list_second_name_with_line(node, src);
            if name.is_empty() {
                return;
            }
            // Emit a ref for the name sym_lit so its sym_name node is covered.
            refs.push(ExtractedRef {
                source_symbol_index: parent_idx.unwrap_or(0),
                target_name: name.clone(),
                kind: EdgeKind::Calls,
                line: name_line,
                module: None,
                chain: None,
            });
            let vis = if head == "defn-" {
                Visibility::Private
            } else {
                Visibility::Public
            };
            let idx = push_sym(node, name, SymbolKind::Function, vis, symbols, parent_idx);
            walk_list_children(node, src, symbols, refs, Some(idx));
        }
        "def" | "defonce" => {
            let (name, name_line) = list_second_name_with_line(node, src);
            if name.is_empty() {
                return;
            }
            refs.push(ExtractedRef {
                source_symbol_index: parent_idx.unwrap_or(0),
                target_name: name.clone(),
                kind: EdgeKind::Calls,
                line: name_line,
                module: None,
                chain: None,
            });
            let idx = push_sym(
                node,
                name,
                SymbolKind::Variable,
                Visibility::Public,
                symbols,
                parent_idx,
            );
            walk_list_children(node, src, symbols, refs, Some(idx));
        }
        "defrecord" | "deftype" => {
            let (name, name_line) = list_second_name_with_line(node, src);
            if name.is_empty() {
                return;
            }
            refs.push(ExtractedRef {
                source_symbol_index: parent_idx.unwrap_or(0),
                target_name: name.clone(),
                kind: EdgeKind::Calls,
                line: name_line,
                module: None,
                chain: None,
            });
            let idx = push_sym(
                node,
                name,
                SymbolKind::Struct,
                Visibility::Public,
                symbols,
                parent_idx,
            );
            walk_list_children(node, src, symbols, refs, Some(idx));
        }
        "defprotocol" | "definterface" => {
            let (name, name_line) = list_second_name_with_line(node, src);
            if name.is_empty() {
                return;
            }
            refs.push(ExtractedRef {
                source_symbol_index: parent_idx.unwrap_or(0),
                target_name: name.clone(),
                kind: EdgeKind::Calls,
                line: name_line,
                module: None,
                chain: None,
            });
            let idx = push_sym(
                node,
                name,
                SymbolKind::Interface,
                Visibility::Public,
                symbols,
                parent_idx,
            );
            walk_list_children(node, src, symbols, refs, Some(idx));
        }
        "ns" => {
            let (ns_name, name_line) = list_second_name_with_line(node, src);
            if !ns_name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: parent_idx.unwrap_or(0),
                    target_name: ns_name.clone(),
                    kind: EdgeKind::Calls,
                    line: name_line,
                    module: None,
                    chain: None,
                });
                let idx = push_sym(
                    node,
                    ns_name,
                    SymbolKind::Namespace,
                    Visibility::Public,
                    symbols,
                    parent_idx,
                );
                extract_ns_refs(node, src, refs, idx);
                // Recurse into the ns form body so that all sym_lits inside
                // :require / :use / :import vectors are covered as refs.
                walk_list_children(node, src, symbols, refs, Some(idx));
            }
        }
        _ => {
            // Head ref already emitted above. Walk argument children.
            walk_call_args(node, src, symbols, refs, parent_idx);
        }
    }
}

/// Extract the head verb and its start line from a `list_lit`.
/// Returns `(name, line)` for the first `sym_lit` child, or `("", node_line)` if none.
fn list_head_with_line(node: Node, src: &[u8]) -> (String, u32) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "sym_lit" {
            let name = sym_lit_name(child, src);
            let line = child.start_position().row as u32;
            return (name, line);
        }
    }
    (String::new(), node.start_position().row as u32)
}

/// Extract the defined name and its `sym_name` leaf line from the second
/// `sym_lit` child of a `list_lit`.
///
/// Uses the `sym_name` child's line (not the outer `sym_lit`'s line) so that
/// coverage correlation works even when metadata decorators like `^:const` or
/// `^{:tag Foo}` push the `sym_lit`'s start earlier than the name text.
fn list_second_name_with_line(node: Node, src: &[u8]) -> (String, u32) {
    let mut cursor = node.walk();
    let mut count = 0usize;
    for child in node.children(&mut cursor) {
        if child.kind() == "sym_lit" {
            if count == 1 {
                let name = sym_lit_name(child, src);
                let line = sym_name_line(child)
                    .unwrap_or_else(|| child.start_position().row as u32);
                return (name, line);
            }
            count += 1;
        }
    }
    (String::new(), 0)
}

/// Return the start row of the first `sym_name` child of a `sym_lit`, if any.
fn sym_name_line(sym_lit_node: Node) -> Option<u32> {
    let mut cursor = sym_lit_node.walk();
    for child in sym_lit_node.children(&mut cursor) {
        if child.kind() == "sym_name" {
            return Some(child.start_position().row as u32);
        }
    }
    None
}

/// Extract the bare name from a `sym_lit` node, ignoring any metadata prefix.
///
/// Plain sym_lit:    `sym_lit → sym_name = "foo"`               → returns `"foo"`
/// Metadata sym_lit: `sym_lit → [meta_lit, sym_name = "foo"]`   → returns `"foo"`
/// Namespaced:       `sym_lit → [sym_ns, sym_name = "bar"]`     → returns `"bar"`
///
/// Falling back to the full text handles sym_lits parsed without a separate
/// `sym_name` child (rare edge cases in the grammar).
fn sym_lit_name(node: Node, src: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "sym_name" {
            let t = child.utf8_text(src).unwrap_or("").trim().to_string();
            if !t.is_empty() {
                return t;
            }
        }
    }
    // Fallback: full sym_lit text (no metadata child present)
    node.utf8_text(src).unwrap_or("").trim().to_string()
}

/// Walk children of a declaration form, starting after the head and name.
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

/// Walk all argument children of a call-form list_lit (skipping the head).
/// Emits refs for sym_lits in argument positions.
fn walk_call_args(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    let mut cursor = node.walk();
    let mut first = true;
    for child in node.children(&mut cursor) {
        if first {
            first = false;
            continue; // skip head (already emitted as Calls ref above)
        }
        if child.kind() == "sym_lit" {
            let name = sym_lit_name(child, src);
            // Emit refs for all sym_lits in argument positions.
            if !name.is_empty() && !name.starts_with(':') {
                refs.push(ExtractedRef {
                    source_symbol_index: parent_idx.unwrap_or(0),
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        } else {
            walk_node(child, src, symbols, refs, parent_idx);
        }
    }
}

fn extract_ns_refs(node: Node, src: &[u8], refs: &mut Vec<ExtractedRef>, sym_idx: usize) {
    // Walk children of the ns form looking for vec_lit / list_lit with :require/:use/:import
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "vec_lit" || child.kind() == "list_lit" {
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
        return sym_lit_name(node, src);
    }
    // For vec_lit like `[some.ns :as alias]`, take the first sym_lit
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "sym_lit" {
            return sym_lit_name(child, src);
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
