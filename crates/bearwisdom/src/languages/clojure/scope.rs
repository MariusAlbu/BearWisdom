// Pure node-shape helpers for Clojure extraction.
//
// All exports are crate-private and depend only on tree-sitter's Node API.
// They form the leaf utility layer of the extractor — every other
// extractor module (extract, method_bodies, reitit) calls down here.
//
// Sections:
//   - Sym/list shape helpers: sym_lit_name, sym_lit_ns, sym_name_line,
//     list_head_with_line, list_second_name_with_line.
//   - Non-callable / skippable / local-binding predicates.
//   - Scope tracking: collect_params_from_vec, collect_binding_names,
//     collect_let_bindings, extend_scope.
//   - Form-specific param collectors: collect_defn_params, collect_fn_params,
//     collect_binding_form_locals, collect_letfn_locals.

use std::collections::HashSet;
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Sym / list shape helpers
// ---------------------------------------------------------------------------

/// Extract the head verb, its namespace qualifier, and its start line from a `list_lit`.
/// Returns `(name, ns, line)` for the first `sym_lit` child, or `("", None, node_line)` if none.
pub(super) fn list_head_with_line(node: Node, src: &[u8]) -> (String, Option<String>, u32) {
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
pub(super) fn list_second_name_with_line(node: Node, src: &[u8]) -> (String, u32) {
    let mut cursor = node.walk();
    let mut count = 0usize;
    for child in node.children(&mut cursor) {
        if child.kind() == "sym_lit" {
            if count == 1 {
                let name = sym_lit_name(child, src);
                let line =
                    sym_name_line(child).unwrap_or_else(|| child.start_position().row as u32);
                return (name, line);
            }
            count += 1;
        }
    }
    (String::new(), 0)
}

/// Return the start row of the first `sym_name` child of a `sym_lit`, if any.
pub(super) fn sym_name_line(sym_lit_node: Node) -> Option<u32> {
    let mut cursor = sym_lit_node.walk();
    for child in sym_lit_node.children(&mut cursor) {
        if child.kind() == "sym_name" {
            return Some(child.start_position().row as u32);
        }
    }
    None
}

/// Extract the bare name from a `sym_lit` node, ignoring any metadata prefix.
pub(super) fn sym_lit_name(node: Node, src: &[u8]) -> String {
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
pub(super) fn sym_lit_ns(node: Node, src: &[u8]) -> Option<String> {
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

// ---------------------------------------------------------------------------
// Skippable-symbol predicates
// ---------------------------------------------------------------------------

/// Tokens that lex as `sym_lit` in tree-sitter but are syntactic
/// operators / special forms / test-framework arrows, not callable
/// predicates. Emitting them as Calls refs always lands them in
/// `unresolved_refs` because there's no defining symbol anywhere.
///
/// Captured set comes from corpus analysis of clojure-babashka's
/// `extractor_bug` bucket:
///   * `.`        — Java interop dot (`.method obj`)
///   * `=>`       — test arrow (midje, expectations)
///   * `else`     — `cond`/`case` keyword fallback when written bare
///                  rather than `:else`
///   * `return`   — JS-leaked source string in test fixtures
///   * `this`     — proxy/reify method receiver. Locals tracker
///                  catches it inside the form, this guard catches
///                  the residue when forms are nested oddly.
///   * `&`        — rest-args separator in `[a b & rest]`. Should be
///                  filtered by the param-vec walk but residual
///                  appearances slip through as call args.
///   * `try` / `catch` / `finally` / `with-open` — exception and
///                  resource special-form keywords. The `catch` arm scopes
///                  the bound exception var and the binding-form arm scopes
///                  `with-open` resource names; these heads themselves are
///                  never callable.
pub(super) fn is_clojure_non_callable_token(name: &str) -> bool {
    matches!(
        name,
        "." | "=>"
            | "else"
            | "return"
            | "this"
            | "&"
            | "try"
            | "catch"
            | "finally"
            | "with-open"
    )
}

/// Combined skip rule for sym_lit nodes that should NOT produce Calls refs.
///
/// Three categories:
///  * **Non-callable tokens** — see `is_clojure_non_callable_token`.
///  * **Logic vars** — `?x`, `?e`, `?node`. core.logic / core.match /
///    datalog pattern vars bound by the macro. Never function calls.
///  * **Syntax-quote gensyms** — `name#`. Inside a `defmacro`, ``foo#``
///    expands to a unique `foo__123__auto__` symbol scoped to the
///    macro expansion; it's a binding, never a callable.
///
/// Used at every sym_lit Calls-ref emission site.
pub(super) fn is_clojure_skippable_symbol(name: &str) -> bool {
    is_clojure_non_callable_token(name)
        || name.starts_with('?')
        || name.ends_with('#')
        || is_clojure_namespace_ref(name)
}

/// True when `name` is a dotted namespace/package reference rather than a
/// callable — `sci.core`, `datascript.db`. Clojure namespace names use interior
/// dots as segment separators; var and function names never contain them. The
/// callable dotted forms are distinguished by their dot position: Java member
/// access is a leading dot (`.method`), a constructor is a trailing dot
/// (`Date.`), and a static-method call carries a `/` qualifier (`Math/abs`, so
/// the bare name has no dot at all). An interior dot with neither marker is a
/// namespace head, which the `:require` clause already models as an Imports
/// target.
fn is_clojure_namespace_ref(name: &str) -> bool {
    match name.find('.') {
        Some(pos) => pos > 0 && pos < name.len() - 1,
        None => false,
    }
}

/// Returns true if `name` is a local binding (unqualified symbol in the locals set).
#[inline]
pub(super) fn is_local(node: Node, src: &[u8], name: &str, locals: &HashSet<String>) -> bool {
    // Namespace-qualified refs (e.g. str/join) are never locals — the module
    // qualifier disambiguates them.
    sym_lit_ns(node, src).is_none() && locals.contains(name)
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
pub(super) fn collect_params_from_vec(node: Node, src: &[u8]) -> HashSet<String> {
    let mut names = HashSet::new();
    collect_binding_names(node, src, &mut names);
    names
}

/// Recursively collect binding names from a pattern node.
pub(super) fn collect_binding_names(node: Node, src: &[u8], names: &mut HashSet<String>) {
    match node.kind() {
        "sym_lit" => {
            let name = sym_lit_name(node, src);
            // Skip & (varargs marker), _ (ignore), keywords, anon fn args
            if !name.is_empty()
                && name != "&"
                && !name.starts_with(':')
                && !name.starts_with('%')
                && !is_clojure_non_callable_token(&name)
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
pub(super) fn collect_let_bindings(vec_node: Node, src: &[u8]) -> HashSet<String> {
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
pub(super) fn extend_scope(
    parent: &HashSet<String>,
    new_names: HashSet<String>,
) -> HashSet<String> {
    if new_names.is_empty() {
        return parent.clone();
    }
    let mut merged = parent.clone();
    merged.extend(new_names);
    merged
}

// ---------------------------------------------------------------------------
// Param / local collection helpers
// ---------------------------------------------------------------------------

/// Collect params for `defn`/`defmacro`/`defrecord`/`deftype` — the `vec_lit`
/// immediately following the name (3rd child of the list).
pub(super) fn collect_defn_params(
    node: Node,
    src: &[u8],
    parent_locals: &HashSet<String>,
) -> HashSet<String> {
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
pub(super) fn collect_fn_params(
    node: Node,
    src: &[u8],
    parent_locals: &HashSet<String>,
) -> HashSet<String> {
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
pub(super) fn collect_binding_form_locals(
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
pub(super) fn collect_letfn_locals(
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

/// Collect the exception var bound by a `catch` clause:
/// `(catch Class e body...)` binds `e` for the body. The first sym_lit is
/// the `catch` head, the second is the exception class (a real type ref),
/// the third is the bound var. Returns the parent scope extended with the
/// bound var name.
pub(super) fn collect_catch_local(
    node: Node,
    src: &[u8],
    parent_locals: &HashSet<String>,
) -> HashSet<String> {
    let mut cursor = node.walk();
    let mut sym_count = 0usize;
    for child in node.children(&mut cursor) {
        if child.kind() == "sym_lit" {
            sym_count += 1;
            // 1 = `catch` head, 2 = exception class, 3 = bound var.
            if sym_count == 3 {
                let name = sym_lit_name(child, src);
                if !name.is_empty() && name != "_" {
                    return extend_scope(parent_locals, HashSet::from([name]));
                }
                break;
            }
        }
    }
    parent_locals.clone()
}
