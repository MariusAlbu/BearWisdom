// Method-body walkers for `reify`, `proxy`, `deftype`/`defrecord`,
// `extend-type`/`extend-protocol`, and `defprotocol`/`definterface`.
//
// Each function handles one declaration shape: it skips the head/name
// boilerplate, collects per-method param locals from the leading vec_lit,
// emits the method name as a Calls ref pointing at the protocol
// definition, then delegates body walking back to `extract::walk_node`.
//
// All entry points are crate-private (`pub(super)`); the dispatcher in
// `extract::process_list` picks the right walker based on the head verb.

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol};
use std::collections::HashSet;
use tree_sitter::Node;

use super::extract::walk_node;
use super::scope::{collect_params_from_vec, extend_scope, sym_lit_name};

/// Walk a `(reify Interface (MethodName [this ...] body...) ...)` form.
///
/// Every `list_lit` child is treated as a method implementation:
///   - collect params from its first `vec_lit` child
///   - walk the rest of the list with those params as locals
/// Other children (sym_lits naming the interface) are walked normally.
pub(super) fn walk_reify_body(
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
        descend_method_or_protocol(child, src, symbols, refs, parent_idx, locals);
    }
}

/// Walk a `(proxy [Super] [ctor-args] (MethodName [params] body...) ...)` form.
///
/// Skips the two mandatory `vec_lit` children (superclass list + ctor args),
/// then walks each method `list_lit` with per-method param scope.
pub(super) fn walk_proxy_body(
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
pub(super) fn walk_with_method_bodies(
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
        descend_method_or_protocol(
            child, src, symbols, refs, parent_idx, field_locals,
        );
    }
}

/// Recursively walk a deftype/defrecord/reify/extend body child, treating
/// any `list_lit` as a protocol method body and recursing into reader
/// conditionals so nested method shapes get the per-method local scope.
/// Without this, `#?@(:cljs [IFn (-invoke [this a b] ...) ...])` leaks
/// the method head as an unresolved Calls ref and the param vec's `a` /
/// `b` get emitted as cross-namespace value lookups.
pub(super) fn descend_method_or_protocol(
    child: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
    locals: &HashSet<String>,
) {
    match child.kind() {
        "list_lit" => {
            walk_method_body(child, src, symbols, refs, parent_idx, locals);
        }
        "vec_lit" | "read_cond_lit" | "splicing_read_cond_lit" => {
            let mut cursor = child.walk();
            for sub in child.children(&mut cursor) {
                descend_method_or_protocol(sub, src, symbols, refs, parent_idx, locals);
            }
        }
        _ => {
            walk_node(child, src, symbols, refs, parent_idx, locals);
        }
    }
}

/// Walk a single method body `(MethodName [params] body...)` with a fresh param scope.
///
/// The method name is emitted as a Calls ref (it names the protocol method being
/// implemented), then params are collected from the first `vec_lit` child, and
/// the body is walked with those params as locals.
pub(super) fn walk_method_body(
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
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
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
pub(super) fn walk_extend_body(
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
pub(super) fn walk_protocol_method_specs(
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
