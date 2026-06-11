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
    EdgeKind, ExtractedDbSet, ExtractedRef, ExtractedRoute, ExtractedSymbol, ExtractionResult,
    SymbolKind, Visibility,
};
use std::collections::HashSet;
use tree_sitter::{Node, Parser};

use super::method_bodies::{
    walk_extend_body, walk_protocol_method_specs, walk_proxy_body, walk_reify_body,
    walk_with_method_bodies,
};
use super::reitit::scan_reitit_routes;
use super::scope::{
    collect_binding_form_locals, collect_catch_local, collect_defn_params, collect_fn_params,
    collect_letfn_locals, collect_params_from_vec, extend_scope, is_clojure_skippable_symbol,
    is_local, list_head_with_line, list_second_name_with_line, sym_lit_name, sym_lit_ns,
    sym_name_line,
};

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

    walk_node(
        tree.root_node(),
        src,
        &mut symbols,
        &mut refs,
        None,
        &locals,
    );

    // Reitit data-driven routes: `["/api" ["/users" {:get user-handler}]]`.
    // A second pass — independent of the call-tracking walker — looks for
    // vec_lits whose first child is a `/`-prefixed string literal and walks
    // their children to emit ExtractedRoute per (path, verb) pair.
    let mut routes: Vec<ExtractedRoute> = Vec::new();
    scan_reitit_routes(tree.root_node(), src, "", &symbols, &mut routes);

    ExtractionResult::with_connectors(
        symbols,
        refs,
        routes,
        Vec::<ExtractedDbSet>::new(),
        tree.root_node().has_error(),
    )
}

// ---------------------------------------------------------------------------
// Tree walk
// ---------------------------------------------------------------------------

pub(super) fn walk_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    locals: &HashSet<String>,
) {
    // Quoted forms — `'foo`, `'(a b c)` — are data, not calls. Their
    // sym_lit children are values to be looked up at runtime by the
    // surrounding code (`(get m 'key)`), never callables. Emitting
    // them as Calls refs always lands them in unresolved_refs and
    // pollutes the dependency graph with phantom edges.
    //
    // Syntax-quote (` ` `) is more nuanced — `~expr` and `~@expr`
    // splice out into live code — so we don't blanket-skip those
    // here. The simple `'foo` quote is the common case driving the
    // `-invoke` / `->X` noise inside deftype method bodies.
    if node.kind() == "quoting_lit" {
        return;
    }
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
        // logic variables (?x, ?e — core.logic / core.match pattern vars
        // bound by the macro, never function calls), and names bound in the
        // current scope.
        if !name.is_empty()
            && !name.starts_with(':')
            && !name.starts_with('%')
            && !is_clojure_skippable_symbol(&name)
            && !is_local(node, src, &name, locals)
        {
            let ns = sym_lit_ns(node, src);
            refs.push(ExtractedRef {
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: parent_idx.unwrap_or(0),
                target_name: name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                col: 0,
                module: ns,
                chain: None,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
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
                && !is_clojure_skippable_symbol(&name)
                && !is_local(child, src, &name, locals)
            {
                let ns = sym_lit_ns(child, src);
                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: parent_idx.unwrap_or(0),
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    col: 0,
                    module: ns,
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
        } else {
            walk_node(child, src, symbols, refs, parent_idx, locals);
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
    if !head.starts_with(':')
        && !head.starts_with('"')
        && !head.starts_with('%')
        && !is_clojure_skippable_symbol(&head)
        && !head_is_local
    {
        refs.push(ExtractedRef {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index: parent_idx.unwrap_or(0),
            target_name: head.clone(),
            kind: EdgeKind::Calls,
            line: head_line,
            col: 0,
            module: head_ns,
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
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
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: parent_idx.unwrap_or(0),
                target_name: name.clone(),
                kind: EdgeKind::Calls,
                line: name_line,
                col: 0,
                module: None,
                chain: None,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
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
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: parent_idx.unwrap_or(0),
                target_name: name.clone(),
                kind: EdgeKind::Calls,
                line: name_line,
                col: 0,
                module: None,
                chain: None,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
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
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: parent_idx.unwrap_or(0),
                target_name: name.clone(),
                kind: EdgeKind::Calls,
                line: name_line,
                col: 0,
                module: None,
                chain: None,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
            let idx = push_sym(
                node,
                name.clone(),
                SymbolKind::Struct,
                Visibility::Public,
                symbols,
                parent_idx,
            );
            // Auto-generated constructors. `defrecord X [a b]` produces both
            // `(->X a b)` (positional) and `(map->X {:a 1 :b 2})` (map-keyword).
            // `deftype X [a b]` only produces `(->X a b)`.
            // Emit them as Function siblings of the Struct so calls resolve.
            push_sym(
                node,
                format!("->{name}"),
                SymbolKind::Function,
                Visibility::Public,
                symbols,
                parent_idx,
            );
            if head == "defrecord" {
                push_sym(
                    node,
                    format!("map->{name}"),
                    SymbolKind::Function,
                    Visibility::Public,
                    symbols,
                    parent_idx,
                );
            }
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
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: parent_idx.unwrap_or(0),
                target_name: name.clone(),
                kind: EdgeKind::Calls,
                line: name_line,
                col: 0,
                module: None,
                chain: None,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
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
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: parent_idx.unwrap_or(0),
                    target_name: ns_name.clone(),
                    kind: EdgeKind::Calls,
                    line: name_line,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
                let idx = push_sym(
                    node,
                    ns_name,
                    SymbolKind::Module,
                    Visibility::Public,
                    symbols,
                    parent_idx,
                );
                extract_ns_refs(node, src, refs, idx);
                walk_list_children(node, src, symbols, refs, Some(idx), locals);
            }
        }
        // Binding forms whose first vec_lit holds `[name expr name expr ...]`
        // pairs (let-style). `if-let` / `when-let` / `if-some` /
        // `when-some` / `when-first` / `dotimes` use a single
        // `[name expr]` pair, which the let-style collector handles
        // correctly because position 0 is still the binding name.
        "let" | "let*" | "loop" | "binding" | "with-redefs" | "with-bindings"
        | "with-local-vars" | "with-open" | "if-let" | "when-let" | "if-some" | "when-some"
        | "when-first" | "dotimes" => {
            let binding_locals = collect_binding_form_locals(node, src, locals);
            walk_list_children_raw(node, src, symbols, refs, parent_idx, &binding_locals, 1);
        }
        // `(catch Class e body...)` — `Class` is a real type reference, `e`
        // is the exception var bound for the body. Scope `e` as a local so
        // its body uses don't leak as Calls refs, then walk the body.
        "catch" => {
            let exc_local = collect_catch_local(node, src, locals);
            walk_list_children_raw(node, src, symbols, refs, parent_idx, &exc_local, 1);
        }
        // `are` from clojure.test — `(are [a b c] expr & values)`. Every
        // name in the first vec is a binding; values that follow are
        // literal data, not bindings. Reuses fn-style first-vec param
        // collection so all positions are taken as locals (let-style
        // pair logic would only catch the even-indexed positions).
        "are" => {
            let binding_locals = collect_fn_params(node, src, locals);
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
        _ if is_custom_def_macro(&head) => {
            // Custom `def*` macro — `(defreq delete)`, `(defroute home [...])`,
            // `(defcomponent MyComp ...)`, etc. The second sym_lit child is
            // the symbol being DEFINED, not a call target. Push it as a
            // symbol of the same project and skip the Calls-ref emission.
            // Mirrors the explicit `defn` / `def` arms above for macros the
            // extractor doesn't know by name.
            let (name, _name_line) = list_second_name_with_line(node, src);
            if !name.is_empty() {
                let idx = push_sym(
                    node,
                    name.clone(),
                    SymbolKind::Function,
                    Visibility::Public,
                    symbols,
                    parent_idx,
                );
                // Walk remaining children but with locals extended to
                // include the just-defined name (avoids self-ref noise
                // when the body uses its own name) and skip the first
                // sym_lit (the name itself).
                let mut new_locals = locals.clone();
                new_locals.insert(name);
                walk_def_macro_body(node, src, symbols, refs, Some(idx), &new_locals);
            } else {
                walk_call_args(node, src, symbols, refs, parent_idx, locals);
            }
        }
        _ => {
            // Head ref already emitted above. Walk argument children.
            walk_call_args(node, src, symbols, refs, parent_idx, locals);
        }
    }
}

/// Heuristic: head looks like a custom `def*`-macro (`defreq`,
/// `defroute`, `defcomponent`, `def-foo`, ...) — names starting with
/// `def` followed by either an uppercase letter, `-`, or another
/// lowercase letter. Excludes `def`, `defn`, `defn-`, `defmacro`,
/// `defmulti`, `defmethod`, `defrecord`, `deftype`, `defprotocol`,
/// `definterface`, `defonce` which are handled by their own match
/// arms above.
fn is_custom_def_macro(head: &str) -> bool {
    if !head.starts_with("def") {
        return false;
    }
    matches!(
        head,
        "def"
            | "defn"
            | "defn-"
            | "defmacro"
            | "defmulti"
            | "defmethod"
            | "defrecord"
            | "deftype"
            | "defprotocol"
            | "definterface"
            | "defonce"
    ) == false
        && head.len() > 3
        && head
            .chars()
            .nth(3)
            .map(|c| c.is_ascii_alphabetic() || c == '-')
            .unwrap_or(false)
}

/// Walk a custom-def-macro body — like `walk_call_args` but skips the
/// first sym_lit child too (it's the name being defined, already
/// pushed as a symbol).
fn walk_def_macro_body(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    locals: &HashSet<String>,
) {
    let mut cursor = node.walk();
    let mut sym_seen = 0usize;
    for child in node.children(&mut cursor) {
        if child.kind() == "sym_lit" {
            sym_seen += 1;
            // Skip head (sym_seen == 1) and the name being defined
            // (sym_seen == 2).
            if sym_seen <= 2 {
                continue;
            }
            // Subsequent sym_lit children — emit as Calls refs.
            let name = sym_lit_name(child, src);
            if !name.is_empty()
                && !name.starts_with(':')
                && !name.starts_with('%')
                && !is_clojure_skippable_symbol(&name)
                && !is_local(child, src, &name, locals)
            {
                let ns = sym_lit_ns(child, src);
                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: parent_idx.unwrap_or(0),
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    col: 0,
                    module: ns,
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
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
                && !is_clojure_skippable_symbol(&name)
                && !is_local(child, src, &name, locals)
            {
                let ns = sym_lit_ns(child, src);
                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: parent_idx.unwrap_or(0),
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    col: 0,
                    module: ns,
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
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
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
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
    // Walk children of the ns form looking for vec_lit / list_lit with
    // :require/:use/:import. A whole require clause may also sit inside a
    // reader conditional — `#?(:clj (:require ...) :cljs ...)` — so descend
    // into read_cond_lit branches and re-run the same scan there.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "vec_lit" | "list_lit" => extract_ns_clause(child, src, refs, sym_idx),
            "read_cond_lit" | "splicing_read_cond_lit" => {
                for branch in reader_conditional_branches(child) {
                    extract_ns_clause(branch, src, refs, sym_idx);
                }
            }
            _ => {}
        }
    }
}

/// Scan one `(:require ...)` / `(:use ...)` / `(:import ...)` clause for
/// import entries. Import entries may be plain `vec_lit`s
/// (`[a.b :refer [foo]]`) or wrapped in a reader conditional
/// (`#?(:clj [a.b :refer [foo]] :cljs [...])`) — both yield Imports refs.
fn extract_ns_clause(clause: Node, src: &[u8], refs: &mut Vec<ExtractedRef>, sym_idx: usize) {
    let mut inner = clause.walk();
    let mut first = true;
    let mut is_import = false;
    for inner_child in clause.children(&mut inner) {
        if inner_child.kind() == "kwd_lit" && first {
            let kw = inner_child.utf8_text(src).unwrap_or("");
            is_import = matches!(kw, ":require" | ":use" | ":import");
            first = false;
            continue;
        }
        if !is_import {
            continue;
        }
        match inner_child.kind() {
            "read_cond_lit" | "splicing_read_cond_lit" => {
                for entry in reader_conditional_branches(inner_child) {
                    emit_import_entry(entry, src, refs, sym_idx);
                }
            }
            _ => emit_import_entry(inner_child, src, refs, sym_idx),
        }
    }
}

/// Emit an Imports ref for a single require entry (`a.b` or
/// `[a.b :refer [foo]]`), plus per-name `:refer` refs when the entry is a vec.
fn emit_import_entry(entry: Node, src: &[u8], refs: &mut Vec<ExtractedRef>, sym_idx: usize) {
    let name = extract_first_sym(entry, src);
    if name.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: sym_idx,
        target_name: name.clone(),
        kind: EdgeKind::Imports,
        line: entry.start_position().row as u32,
        col: 0,
        module: None,
        chain: None,
        byte_offset: entry.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
    // Per-name `:refer [n1 n2]` imports — emit each referred symbol as its
    // own Imports ref so the resolver can match unqualified call sites
    // (`(match? a b)`) back to their source namespace without static
    // analysis of what's `:refer :all`'d. Module on these refs IS the source
    // namespace, which infer_external_namespace then surfaces.
    if entry.kind() == "vec_lit" {
        collect_refer_names(entry, src, &name, sym_idx, refs);
    }
}

/// Yield the data branches of a reader-conditional node, skipping the
/// `:clj` / `:cljs` / `:default` platform tags. A `#?(:clj A :cljs B)` form
/// is a flat sequence of alternating `kwd_lit` tags and value nodes; the
/// values are the import entries (or clauses) we want.
fn reader_conditional_branches(node: Node) -> Vec<Node> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() && child.kind() != "kwd_lit" {
            out.push(child);
        }
    }
    out
}

/// Inside a `[ns :refer [n1 n2 ...]]` vec, find the `:refer` keyword
/// followed by a `vec_lit` and emit each contained sym_lit as an Imports
/// ref keyed to `ns`. `:refer :all` leaves the refer-vec unset and is a
/// wildcard — handled by the existing namespace-level Imports ref.
fn collect_refer_names(
    vec_node: Node,
    src: &[u8],
    ns_name: &str,
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = vec_node.walk();
    let mut see_refer = false;
    for child in vec_node.children(&mut cursor) {
        if child.kind() == "kwd_lit" {
            let kw = child.utf8_text(src).unwrap_or("");
            see_refer = kw == ":refer";
            continue;
        }
        if see_refer && child.kind() == "vec_lit" {
            let mut inner = child.walk();
            for sym in child.children(&mut inner) {
                if sym.kind() != "sym_lit" {
                    continue;
                }
                let name = sym_lit_name(sym, src);
                if name.is_empty() {
                    continue;
                }
                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: sym_idx,
                    target_name: name,
                    kind: EdgeKind::Imports,
                    line: sym.start_position().row as u32,
                    col: 0,
                    module: Some(ns_name.to_string()),
                    chain: None,
                    byte_offset: sym.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
            return;
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
// Symbol push
// ---------------------------------------------------------------------------

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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
    idx
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;
