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
// SCOPE TRACKING:
//   Local bindings are collected and suppressed from Calls refs.
//   Forms that introduce locals:
//     - `defn`/`defn-`/`defmacro`/`fn` parameter vectors: [x y z]
//     - `let`/`loop`/`letfn`/`binding`/`with-redefs`/`doseq`/`for`
//       binding vectors: [name expr name2 expr2]
//     - Destructuring in binding positions: {:keys [a b]} [a b]
//   A sym_lit whose target_name is in the current locals set (and has no
//   namespace qualifier) is not emitted as a Calls ref.
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
use std::collections::HashSet;
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
    let locals = HashSet::new();

    walk_node(tree.root_node(), src, &mut symbols, &mut refs, None, &locals);

    ExtractionResult::new(symbols, refs, tree.root_node().has_error())
}

// ---------------------------------------------------------------------------
// Scope helpers
// ---------------------------------------------------------------------------

/// Collect all locally-bound names from a `vec_lit` parameter/binding node.
///
/// Handles:
///   - Plain sym_lits: `[request respond raise]` → {"request","respond","raise"}
///   - Map destructuring: `{:keys [a b] :as m}` → {"a","b","m"}
///   - Vector destructuring: `[a b & rest]` → {"a","b","rest"}
///   - Nested patterns are not recursed — only first-level names.
fn collect_params_from_vec(node: Node, src: &[u8]) -> HashSet<String> {
    let mut names = HashSet::new();
    collect_binding_names(node, src, &mut names);
    names
}

/// Recursively collect binding names from a pattern node.
fn collect_binding_names(node: Node, src: &[u8], names: &mut HashSet<String>) {
    match node.kind() {
        "sym_lit" => {
            let name = sym_lit_name(node, src);
            // Skip & (varargs marker), _ (ignore), keywords, anon fn args
            if !name.is_empty()
                && name != "&"
                && !name.starts_with(':')
                && !name.starts_with('%')
                && !name.starts_with('"')
            {
                names.insert(name);
            }
        }
        "vec_lit" => {
            // [a b & rest] — collect all sym_lits at this level
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_binding_names(child, src, names);
            }
        }
        "map_lit" => {
            // {:keys [a b] :strs [c] :syms [d] :as m} — collect :keys/:strs/:syms vectors
            // and the :as alias
            let mut cursor = node.walk();
            let children: Vec<Node> = node.children(&mut cursor).collect();
            let mut i = 0;
            while i < children.len() {
                let child = children[i];
                if child.kind() == "kwd_lit" {
                    let kw = child.utf8_text(src).unwrap_or("").trim();
                    if matches!(kw, ":keys" | ":strs" | ":syms") {
                        // Next child should be a vec_lit of names
                        if let Some(next) = children.get(i + 1) {
                            if next.kind() == "vec_lit" {
                                let mut vc = next.walk();
                                for inner in next.children(&mut vc) {
                                    if inner.kind() == "sym_lit" {
                                        let n = sym_lit_name(inner, src);
                                        if !n.is_empty() {
                                            names.insert(n);
                                        }
                                    }
                                }
                                i += 2;
                                continue;
                            }
                        }
                    } else if kw == ":as" {
                        // :as alias — next sym_lit is the alias name
                        if let Some(next) = children.get(i + 1) {
                            if next.kind() == "sym_lit" {
                                let n = sym_lit_name(*next, src);
                                if !n.is_empty() {
                                    names.insert(n);
                                }
                                i += 2;
                                continue;
                            }
                        }
                    } else if kw == ":or" {
                        // :or default map — skip it entirely (values are defaults, not bindings)
                        i += 2;
                        continue;
                    }
                }
                i += 1;
            }
        }
        "meta_lit" => {
            // ^TypeHint name — the actual binding name is the last child sym_lit.
            // e.g. ^Request base-request → collect `base-request`.
            let mut cursor = node.walk();
            let children: Vec<Node> = node.children(&mut cursor).collect();
            // The annotated form is the last named child.
            if let Some(last) = children.iter().rev().find(|c| c.is_named()) {
                collect_binding_names(*last, src, names);
            }
        }
        _ => {
            // Other node kinds (literals, etc.) — no bindings to collect
        }
    }
}

/// Collect let-style binding names from a `vec_lit` binding vector.
///
/// In `(let [a 1, b 2, {:keys [c d]} m] ...)` the binding vector has pairs:
/// `[pattern expr pattern expr ...]`. We collect names from even-indexed
/// (0, 2, 4, ...) positions which are the binding targets.
fn collect_let_bindings(vec_node: Node, src: &[u8]) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut cursor = vec_node.walk();
    let children: Vec<Node> = vec_node.children(&mut cursor).collect();
    let mut i = 0;
    while i < children.len() {
        let child = children[i];
        // Skip punctuation tokens (brackets, commas, whitespace)
        if child.is_named() {
            collect_binding_names(child, src, &mut names);
            // Skip the value expression (the next named child)
            // Fast path: advance past the immediate next named sibling
            i += 1;
            // Skip one value expression (may be multiple raw tokens)
            while i < children.len() && !children[i].is_named() {
                i += 1;
            }
            // Now skip the value node itself
            if i < children.len() {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    names
}

/// Merge a new scope into a cloned set (so the parent scope is unaffected).
fn extend_scope(parent: &HashSet<String>, new_names: HashSet<String>) -> HashSet<String> {
    if new_names.is_empty() {
        return parent.clone();
    }
    let mut merged = parent.clone();
    merged.extend(new_names);
    merged
}

// ---------------------------------------------------------------------------
// Tree walk
// ---------------------------------------------------------------------------

fn walk_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    locals: &HashSet<String>,
) {
    if node.kind() == "list_lit" {
        // process_list handles both symbol extraction and child recursion for list_lits.
        process_list(node, src, symbols, refs, parent_idx, locals);
        return;
    }
    if node.kind() == "sym_lit" {
        // When walk_node is called directly on a sym_lit (e.g. from walk_list_children
        // for body values like `db/tx0`), emit a ref for it here.
        let name = sym_lit_name(node, src);
        // Skip keywords (:foo), anonymous fn args (%, %1, %2, %&), gensyms,
        // and names bound in the current scope.
        if !name.is_empty()
            && !name.starts_with(':')
            && !name.starts_with('%')
            && !is_local(node, src, &name, locals)
        {
            let ns = sym_lit_ns(node, src);
            refs.push(ExtractedRef {
                source_symbol_index: parent_idx.unwrap_or(0),
                target_name: name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module: ns,
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
            if !name.is_empty()
                && !name.starts_with(':')
                && !name.starts_with('%')
                && !is_local(child, src, &name, locals)
            {
                let ns = sym_lit_ns(child, src);
                refs.push(ExtractedRef {
                    source_symbol_index: parent_idx.unwrap_or(0),
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    module: ns,
                    chain: None,
                });
            }
        } else {
            walk_node(child, src, symbols, refs, parent_idx, locals);
        }
    }
}

/// Returns true if `name` is a local binding (unqualified symbol in the locals set).
#[inline]
fn is_local(node: Node, src: &[u8], name: &str, locals: &HashSet<String>) -> bool {
    // Namespace-qualified refs (e.g. str/join) are never locals — the module
    // qualifier disambiguates them.
    sym_lit_ns(node, src).is_none() && locals.contains(name)
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
    locals: &HashSet<String>,
) {
    let (head, head_ns, head_line) = list_head_with_line(node, src);
    if head.is_empty() {
        // No sym_lit head — classify by first named child and walk children.
        let first_named = first_named_child_kind(node);
        match first_named.as_deref() {
            // Multi-arity function clause: ([x] body) — collect params then walk body
            Some("vec_lit") => {
                let param_vec = first_named_child_node(node);
                let new_locals = if let Some(pv) = param_vec {
                    extend_scope(locals, collect_params_from_vec(pv, src))
                } else {
                    locals.clone()
                };
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    walk_node(child, src, symbols, refs, parent_idx, &new_locals);
                }
            }
            // Keyword-headed ns clause or reader-conditional — walk all children
            Some("kwd_lit") | Some("read_cond_lit") | Some("splicing_read_cond_lit") => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    walk_node(child, src, symbols, refs, parent_idx, locals);
                }
            }
            _ => {}
        }
        return;
    }

    // Emit a ref for the head sym_lit (the declaration keyword or call verb),
    // unless it resolves to a local binding (e.g. `(options :key val)` where
    // `options` is a parameter used as a lookup function).
    let head_is_local = head_ns.is_none() && locals.contains(&head);
    if !head.starts_with(':') && !head.starts_with('"') && !head.starts_with('%') && !head_is_local {
        refs.push(ExtractedRef {
            source_symbol_index: parent_idx.unwrap_or(0),
            target_name: head.clone(),
            kind: EdgeKind::Calls,
            line: head_line,
            module: head_ns,
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
            // Collect params from the parameter vector (3rd child after defn + name).
            let param_locals = collect_defn_params(node, src, locals);
            walk_list_children(node, src, symbols, refs, Some(idx), &param_locals);
        }
        "fn" => {
            // Anonymous function: (fn [x y] body) or (fn name [x y] body)
            // Collect params from the parameter vector, then walk body.
            let param_locals = collect_fn_params(node, src, locals);
            // No symbol emitted for anonymous fn; walk children (skip head).
            walk_list_children_raw(node, src, symbols, refs, parent_idx, &param_locals, 1);
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
            walk_list_children(node, src, symbols, refs, Some(idx), locals);
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
            // Collect field names from the fields vector (3rd child after head+name).
            let field_locals = collect_defn_params(node, src, locals);
            // Walk non-method children with field scope; walk method bodies with
            // per-method param scope (each list_lit child after the fields vec is a
            // protocol method implementation: (MethodName [this field...] body...)).
            walk_with_method_bodies(node, src, symbols, refs, Some(idx), &field_locals);
        }
        "reify" => {
            // (reify Interface (MethodName [this ...] body...) ...)
            // No declared name; method list_lit children each carry their own param scope.
            walk_reify_body(node, src, symbols, refs, parent_idx, locals);
        }
        "proxy" => {
            // (proxy [SuperClass] [ctor-args] (MethodName [this ...] body...) ...)
            // Skip head + two vec_lits, then treat remaining list_lits as method bodies.
            walk_proxy_body(node, src, symbols, refs, parent_idx, locals);
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
            // Also emit symbols for each protocol method spec.
            // Each method spec is a list_lit whose head is the method name:
            //   (read-session [store key] "doc")
            //   (write-session [store key data] "doc")
            extract_protocol_methods(node, src, symbols, Some(idx));
            // Walk protocol body: each list_lit child is a method spec whose params
            // should be scoped (not emitted as refs).
            walk_protocol_method_specs(node, src, symbols, refs, Some(idx), locals);
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
                walk_list_children(node, src, symbols, refs, Some(idx), locals);
            }
        }
        // Binding forms — collect locals from binding vector before walking body
        "let" | "let*" | "loop" | "binding" | "with-redefs" | "with-bindings"
        | "with-local-vars" => {
            let binding_locals = collect_binding_form_locals(node, src, locals);
            walk_list_children_raw(node, src, symbols, refs, parent_idx, &binding_locals, 1);
        }
        "letfn" => {
            let letfn_locals = collect_letfn_locals(node, src, locals);
            walk_list_children_raw(node, src, symbols, refs, parent_idx, &letfn_locals, 1);
        }
        "doseq" | "for" => {
            // Same shape as let: binding vector then body
            let binding_locals = collect_binding_form_locals(node, src, locals);
            walk_list_children_raw(node, src, symbols, refs, parent_idx, &binding_locals, 1);
        }
        _ => {
            // Head ref already emitted above. Walk argument children.
            walk_call_args(node, src, symbols, refs, parent_idx, locals);
        }
    }
}

// ---------------------------------------------------------------------------
// Param / local collection helpers
// ---------------------------------------------------------------------------

/// Collect params for `defn`/`defmacro`/`defrecord`/`deftype` — the `vec_lit`
/// immediately following the name (3rd child of the list).
fn collect_defn_params(node: Node, src: &[u8], parent_locals: &HashSet<String>) -> HashSet<String> {
    // Children: ( defn name [params...] body... )
    // We want the first vec_lit child after the head+name.
    let mut cursor = node.walk();
    let mut sym_count = 0usize;
    for child in node.children(&mut cursor) {
        if child.kind() == "sym_lit" {
            sym_count += 1;
            if sym_count == 2 {
                // This is the name — next named sibling should be vec_lit
                continue;
            }
        }
        if sym_count >= 2 && child.kind() == "vec_lit" {
            return extend_scope(parent_locals, collect_params_from_vec(child, src));
        }
    }
    parent_locals.clone()
}

/// Collect params for `fn` — handles both `(fn [x] body)` and `(fn name [x] body)`.
fn collect_fn_params(node: Node, src: &[u8], parent_locals: &HashSet<String>) -> HashSet<String> {
    let mut cursor = node.walk();
    let mut past_head = false;
    for child in node.children(&mut cursor) {
        if !past_head {
            // Skip the `fn` head itself
            if child.kind() == "sym_lit" {
                past_head = true;
            }
            continue;
        }
        match child.kind() {
            "vec_lit" => {
                return extend_scope(parent_locals, collect_params_from_vec(child, src));
            }
            "sym_lit" => {
                // Named fn: (fn name [x] body) — skip the name, keep going for vec_lit
                continue;
            }
            _ => {}
        }
    }
    parent_locals.clone()
}

/// Collect let-style binding locals: (let [a expr b expr] ...)
/// Returns parent scope extended with new binding names.
fn collect_binding_form_locals(
    node: Node,
    src: &[u8],
    parent_locals: &HashSet<String>,
) -> HashSet<String> {
    let mut cursor = node.walk();
    let mut past_head = false;
    for child in node.children(&mut cursor) {
        if !past_head {
            if child.kind() == "sym_lit" {
                past_head = true;
            }
            continue;
        }
        if child.kind() == "vec_lit" {
            return extend_scope(parent_locals, collect_let_bindings(child, src));
        }
    }
    parent_locals.clone()
}

/// Collect letfn binding names: (letfn [(helper [x] x)] body)
/// Each element of the binding vector is a list_lit whose first sym_lit is the name.
fn collect_letfn_locals(
    node: Node,
    src: &[u8],
    parent_locals: &HashSet<String>,
) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut cursor = node.walk();
    let mut past_head = false;
    for child in node.children(&mut cursor) {
        if !past_head {
            if child.kind() == "sym_lit" {
                past_head = true;
            }
            continue;
        }
        if child.kind() == "vec_lit" {
            // Each element is a list_lit: (name [args] body)
            let mut vc = child.walk();
            for fn_form in child.children(&mut vc) {
                if fn_form.kind() == "list_lit" {
                    let (fname, _, _) = list_head_with_line(fn_form, src);
                    // For letfn, the head IS the function name (not a verb)
                    // Actually letfn forms are (name [args] body) — head is the fn name
                    if !fname.is_empty() {
                        names.insert(fname);
                    }
                }
            }
            break;
        }
    }
    extend_scope(parent_locals, names)
}

// ---------------------------------------------------------------------------
// reify / proxy / deftype method-body helpers
// ---------------------------------------------------------------------------

/// Walk a `(reify Interface (MethodName [this ...] body...) ...)` form.
///
/// Every `list_lit` child is treated as a method implementation:
///   - collect params from its first `vec_lit` child
///   - walk the rest of the list with those params as locals
/// Other children (sym_lits naming the interface) are walked normally.
fn walk_reify_body(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    locals: &HashSet<String>,
) {
    let mut cursor = node.walk();
    let mut past_head = false;
    for child in node.children(&mut cursor) {
        if !past_head {
            // Skip the `reify` head sym_lit itself (already emitted as a Calls ref).
            if child.kind() == "sym_lit" {
                past_head = true;
            }
            continue;
        }
        if child.kind() == "list_lit" {
            // Method form: (MethodName [params] body...)
            walk_method_body(child, src, symbols, refs, parent_idx, locals);
        } else {
            walk_node(child, src, symbols, refs, parent_idx, locals);
        }
    }
}

/// Walk a `(proxy [Super] [ctor-args] (MethodName [params] body...) ...)` form.
///
/// Skips the two mandatory `vec_lit` children (superclass list + ctor args),
/// then walks each method `list_lit` with per-method param scope.
fn walk_proxy_body(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    locals: &HashSet<String>,
) {
    let mut cursor = node.walk();
    let mut past_head = false;
    let mut vec_skipped = 0usize;
    for child in node.children(&mut cursor) {
        if !past_head {
            if child.kind() == "sym_lit" {
                past_head = true;
            }
            continue;
        }
        if vec_skipped < 2 && child.kind() == "vec_lit" {
            // First two vec_lits: [SuperClass] and [ctor-args] — skip them.
            vec_skipped += 1;
            continue;
        }
        if child.kind() == "list_lit" {
            walk_method_body(child, src, symbols, refs, parent_idx, locals);
        } else {
            walk_node(child, src, symbols, refs, parent_idx, locals);
        }
    }
}

/// Walk a `defrecord`/`deftype` body where `list_lit` children after the fields
/// vec are protocol method implementations `(MethodName [this f...] body...)`.
///
/// Non-method children (sym_lits naming protocols, keyword options) are walked
/// with the field-level scope so field names are suppressed there too.
fn walk_with_method_bodies(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    field_locals: &HashSet<String>,
) {
    let mut cursor = node.walk();
    let mut skip = 2usize; // skip head (defrecord/deftype) and name sym_lit
    let mut past_fields = false;
    for child in node.children(&mut cursor) {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        if !past_fields && child.kind() == "vec_lit" {
            // This is the fields vec — already consumed into field_locals; skip it.
            past_fields = true;
            continue;
        }
        if child.kind() == "list_lit" {
            // Protocol method implementation: (MethodName [this ...] body...)
            walk_method_body(child, src, symbols, refs, parent_idx, field_locals);
        } else {
            walk_node(child, src, symbols, refs, parent_idx, field_locals);
        }
    }
}

/// Walk a single method body `(MethodName [params] body...)` with a fresh param scope.
///
/// The method name is emitted as a Calls ref (it names the protocol method being
/// implemented), then params are collected from the first `vec_lit` child, and
/// the body is walked with those params as locals.
fn walk_method_body(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    outer_locals: &HashSet<String>,
) {
    let mut cursor = node.walk();
    let mut past_head = false;
    let mut method_locals = outer_locals.clone();
    let mut params_collected = false;

    for child in node.children(&mut cursor) {
        if !past_head {
            // The method-name sym_lit — emit it as a Calls ref (resolves to the
            // protocol method definition) but don't treat it as a local.
            if child.kind() == "sym_lit" {
                let name = sym_lit_name(child, src);
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
                past_head = true;
            }
            continue;
        }
        if !params_collected && child.kind() == "vec_lit" {
            // First vec_lit after the method name = parameter list [this ...]
            let params = collect_params_from_vec(child, src);
            method_locals = extend_scope(outer_locals, params);
            params_collected = true;
            // Don't recurse into the param vec itself — those are declarations, not refs.
            continue;
        }
        // Body expressions — walk with the method-scoped locals.
        walk_node(child, src, symbols, refs, parent_idx, &method_locals);
    }
}

/// Walk `(extend-type TypeName Protocol (method [params] body...) ...)` or
/// `(extend-protocol Protocol TypeName (method [params] body...) ...)`.
///
/// After the head sym_lit, alternates between sym_lits (type/protocol names)
/// and list_lit method implementations. All list_lits get per-method param scope.
fn walk_extend_body(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    locals: &HashSet<String>,
) {
    let mut cursor = node.walk();
    let mut past_head = false;
    for child in node.children(&mut cursor) {
        if !past_head {
            if child.kind() == "sym_lit" {
                past_head = true;
            }
            continue;
        }
        if child.kind() == "list_lit" {
            walk_method_body(child, src, symbols, refs, parent_idx, locals);
        } else {
            walk_node(child, src, symbols, refs, parent_idx, locals);
        }
    }
}

/// Walk the body of a `defprotocol`/`definterface` form, treating each list_lit
/// child as a method spec whose param names are scoped (suppressed as refs).
///
/// Protocol method specs: `(method-name [arg1 arg2] "optional doc string")`.
/// We scope the params from the vec_lit so they don't appear as unresolved refs.
fn walk_protocol_method_specs(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    locals: &HashSet<String>,
) {
    let mut cursor = node.walk();
    let mut skip = 2usize; // skip head + protocol name
    for child in node.children(&mut cursor) {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        if child.kind() == "list_lit" {
            // Method spec: (method-name [params] "doc") — scope params, don't walk body
            // as refs because spec bodies are doc strings only.
            walk_method_body(child, src, symbols, refs, parent_idx, locals);
        } else {
            walk_node(child, src, symbols, refs, parent_idx, locals);
        }
    }
}

// ---------------------------------------------------------------------------
// Child-walking helpers
// ---------------------------------------------------------------------------

/// Walk children of a declaration form, starting after the head and name.
fn walk_list_children(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    locals: &HashSet<String>,
) {
    let mut cursor = node.walk();
    let mut skip = 2usize; // skip head (`defn`) and name
    for child in node.children(&mut cursor) {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        walk_node(child, src, symbols, refs, parent_idx, locals);
    }
}

/// Walk children starting after the first N children (by raw child index, not named-only).
fn walk_list_children_raw(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    locals: &HashSet<String>,
    skip: usize,
) {
    let mut cursor = node.walk();
    let mut skipped = 0usize;
    for child in node.children(&mut cursor) {
        if skipped < skip {
            skipped += 1;
            continue;
        }
        walk_node(child, src, symbols, refs, parent_idx, locals);
    }
}

/// Walk all argument children of a call-form list_lit (skipping the head).
/// Emits refs for sym_lits in argument positions that are not locals.
fn walk_call_args(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    locals: &HashSet<String>,
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
            if !name.is_empty()
                && !name.starts_with(':')
                && !name.starts_with('%')
                && !is_local(child, src, &name, locals)
            {
                let ns = sym_lit_ns(child, src);
                refs.push(ExtractedRef {
                    source_symbol_index: parent_idx.unwrap_or(0),
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    module: ns,
                    chain: None,
                });
            }
        } else {
            walk_node(child, src, symbols, refs, parent_idx, locals);
        }
    }
}

// ---------------------------------------------------------------------------
// Utility helpers for non-sym-headed list_lits
// ---------------------------------------------------------------------------

fn first_named_child_kind(node: Node) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            return Some(child.kind().to_owned());
        }
    }
    None
}

fn first_named_child_node(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            return Some(child);
        }
    }
    None
}

/// Emit a `Function` symbol for each protocol method spec inside a `defprotocol`
/// or `definterface` form.
///
/// Protocol method specs have the shape:
///   `(method-name [arg1 arg2] "optional doc")`
///   `(method-name [arg1] [arg1 arg2] "doc")`  — multi-arity
///
/// We emit one symbol per distinct method name using the line of its first
/// occurrence. The method-name sym_lit is the head of a list_lit child.
fn extract_protocol_methods(
    protocol_node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_idx: Option<usize>,
) {
    let mut cursor = protocol_node.walk();
    // Track emitted names to avoid duplicate symbols from multi-arity specs.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for child in protocol_node.children(&mut cursor) {
        if child.kind() != "list_lit" {
            continue;
        }
        let (method_name, _, method_line) = list_head_with_line(child, src);
        if method_name.is_empty() || method_name.starts_with(':') || seen.contains(&method_name) {
            continue;
        }
        seen.insert(method_name.clone());
        // Use the child node's start line for the symbol; the head gives the name.
        let mut sym = ExtractedSymbol {
            qualified_name: method_name.clone(),
            name: method_name,
            kind: SymbolKind::Function,
            visibility: Some(Visibility::Public),
            start_line: method_line,
            end_line: child.end_position().row as u32,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: parent_idx,
        };
        // Try to build a signature from the first vec_lit child (params).
        let mut inner = child.walk();
        for ic in child.children(&mut inner) {
            if ic.kind() == "vec_lit" {
                let sig_text = ic.utf8_text(src).unwrap_or("").trim().to_string();
                if !sig_text.is_empty() {
                    sym.signature = Some(sig_text);
                }
                break;
            }
        }
        symbols.push(sym);
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

// ---------------------------------------------------------------------------
// sym_lit node helpers
// ---------------------------------------------------------------------------

/// Extract the head verb, its namespace qualifier, and its start line from a `list_lit`.
/// Returns `(name, ns, line)` for the first `sym_lit` child, or `("", None, node_line)` if none.
fn list_head_with_line(node: Node, src: &[u8]) -> (String, Option<String>, u32) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "sym_lit" {
            let name = sym_lit_name(child, src);
            let ns = sym_lit_ns(child, src);
            let line = child.start_position().row as u32;
            return (name, ns, line);
        }
    }
    (String::new(), None, node.start_position().row as u32)
}

/// Extract the defined name and its `sym_name` leaf line from the second
/// `sym_lit` child of a `list_lit`.
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
    // Fallback: full sym_lit text (no metadata child present).
    let full = node.utf8_text(src).unwrap_or("").trim().to_string();
    if let Some(pos) = full.find('/') {
        full[pos + 1..].to_string()
    } else {
        full
    }
}

/// Extract the namespace qualifier from a `sym_lit` node, if present.
fn sym_lit_ns(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "sym_ns" {
            let t = child.utf8_text(src).unwrap_or("").trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    // Fallback: if there is no sym_ns child but the full text has a `/`,
    // treat the part before `/` as the namespace.
    let full = node.utf8_text(src).unwrap_or("").trim();
    if let Some(pos) = full.find('/') {
        let ns = full[..pos].trim();
        if !ns.is_empty() {
            return Some(ns.to_string());
        }
    }
    None
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
