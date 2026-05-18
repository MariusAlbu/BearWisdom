// =============================================================================
// javascript/globals.rs — global-binding harvest for the JavaScript extractor
//
// Classic browser-distributed libraries (`<script src="jquery.js">`,
// `angular.js`, `bootstrap.js`, …) wrap their bodies in an IIFE and
// install their public API by assigning to the global object:
//
//     (function(root, factory) {
//         /* … */
//         root.jQuery = root.$ = factory();
//     })(typeof window !== "undefined" ? window : this, function() { … });
//
// Without special handling, a vanilla extract walks the IIFE body and
// emits function-scoped symbols that no cross-file resolver can match
// against `$` / `jQuery` calls in project code. This module fishes out
// those global assignments and emits them as file-top-level symbols so
// the Tier-1 resolver finds them alongside ordinary top-level
// declarations.
//
// Also owns the post-traversal `type_identifier` scan that catches
// JSDoc-annotated heritage and other type-name leaves the main walker
// misses.
//
// Recognised LHS shapes (object part of `member_expression` or
// `subscript_expression`):
//
//     * `identifier` with name in {window, global, globalThis, self, root}
//     * `this`  (top-level `this.X = …` in sloppy-mode UMD wrappers)
//     * IIFE parameter name that was bound to `window` / `global` via the
//       IIFE's argument list (handled implicitly: all IIFEs have their
//       first parameter treated as a global receiver)
// =============================================================================

use super::helpers::node_text;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol as Sym, SymbolKind, Visibility,
};
use std::collections::HashMap;
use tree_sitter::Node;

/// Recursively scan ALL descendants of `node` for `type_identifier` nodes and
/// emit a `TypeRef` for each one found.
///
/// JavaScript has no type system, so hits are rare (JSDoc-annotated bindings,
/// class heritage identifiers), but the scan is cheap and ensures parity with
/// the TypeScript extractor.
pub(super) fn scan_all_type_identifiers(
    node: Node,
    src: &[u8],
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" && child.is_named() {
            let name = node_text(child, src);
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
        }
        // Recurse into ALL children regardless.
        scan_all_type_identifiers(child, src, sym_idx, refs);
    }
}

pub(super) fn harvest_top_level_globals(root: Node, src: &[u8], symbols: &mut Vec<Sym>) {
    // Pass 1: top-level statements (window.X, IIFE root.X, this.X).
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        scan_statement_for_globals(child, src, &[], symbols);
    }
    // Pass 2: AngularJS DI registrations anywhere in the AST.
    // Patterns like `angular.module(...).service('Upload', ...)` or
    // `ngFileUpload.factory('Foo', [...])` register DI tokens that are
    // consumed across files. Without this, consumer code referencing
    // `Upload.upload(...)` leaves `Upload` unresolved.
    scan_angular_registrations(root, src, symbols);
}

/// AngularJS module-registration methods. Map each to the SymbolKind that
/// best describes what consumers can do with the DI token:
///   - service/factory/provider/component/controller → class-like (has
///     methods, often used as chain receivers → `Upload.upload(...)`).
///   - directive/filter → function-like (invoked directly or as a tag).
///   - value/constant → variable (opaque value).
///   - decorator/run/config → not DI tokens; skip.
fn angular_registration_kind(method: &str) -> Option<SymbolKind> {
    match method {
        "service" | "factory" | "provider" | "component" | "controller" => {
            Some(SymbolKind::Class)
        }
        "directive" | "filter" => Some(SymbolKind::Function),
        "value" | "constant" => Some(SymbolKind::Variable),
        _ => None,
    }
}

/// Walk the AST recursively and emit a top-level symbol for each AngularJS
/// DI registration of the form `X.service('Name', …)`, `.factory(...)`,
/// `.filter(...)`, etc. The string-literal first argument becomes the
/// symbol name; the SymbolKind is chosen by `angular_registration_kind`.
fn scan_angular_registrations(node: Node, src: &[u8], symbols: &mut Vec<Sym>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "call_expression" {
            try_emit_angular_registration(&child, src, symbols);
        }
        scan_angular_registrations(child, src, symbols);
    }
}

fn try_emit_angular_registration(call: &Node, src: &[u8], symbols: &mut Vec<Sym>) {
    let Some(func) = call.child_by_field_name("function") else { return };
    if func.kind() != "member_expression" {
        return;
    }
    let Some(prop) = func.child_by_field_name("property") else { return };
    let method = node_text(prop, src);
    let Some(kind) = angular_registration_kind(&method) else { return };

    let Some(args) = call.child_by_field_name("arguments") else { return };
    let mut acursor = args.walk();
    let Some(first_arg) = args.named_children(&mut acursor).next() else { return };
    if first_arg.kind() != "string" {
        return;
    }
    let Some(name) = string_literal_value(first_arg, src) else { return };
    push_typed_global_symbol(&name, kind, call, symbols);
}

/// Unwrap a string-literal node's text into its content, rejecting empty
/// strings and content that doesn't look like an identifier/DI-token name
/// (to avoid polluting the index with arbitrary string literals).
fn string_literal_value(node: Node, src: &[u8]) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    // Prefer the inner string_fragment node (tree-sitter-javascript parses
    // "foo" as string { string_fragment "foo" }).
    let mut cursor = node.walk();
    let content = node
        .named_children(&mut cursor)
        .find(|c| c.kind() == "string_fragment")
        .map(|c| node_text(c, src))
        .unwrap_or_else(|| {
            let raw = node_text(node, src);
            raw.trim_matches(|c| c == '"' || c == '\'').to_string()
        });
    if content.is_empty() {
        return None;
    }
    // Accept names that look like identifiers or DI tokens ($http, ng-click,
    // etc.). Reject anything with whitespace / quotes / slashes to avoid
    // stuffing arbitrary literals ("api/products", "/path/to") in symbols.
    let ok = content
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '-'));
    if !ok {
        return None;
    }
    Some(content)
}

fn push_typed_global_symbol(
    name: &str,
    kind: SymbolKind,
    anchor: &Node,
    symbols: &mut Vec<Sym>,
) {
    // Dedup: if a top-level symbol with the same (name, kind) already exists,
    // skip. But allow multiple kinds for the same name (a name may exist as
    // both Class (service) and Variable (window alias)).
    if symbols
        .iter()
        .any(|s| s.name == name && s.parent_index.is_none() && s.kind == kind)
    {
        return;
    }
    let line = anchor.start_position().row as u32;
    symbols.push(Sym {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
}

/// Examine one statement node (or the body of a recursively-walked IIFE)
/// for global assignments. `alias_globals` is the set of identifier names
/// that should be treated as "the global object" inside the current
/// scope — starts empty at the file level and is populated with the
/// IIFE's first parameter name when we descend into one.
fn scan_statement_for_globals(
    node: Node,
    src: &[u8],
    alias_globals: &[String],
    symbols: &mut Vec<Sym>,
) {
    match node.kind() {
        "expression_statement" => {
            if let Some(inner) = node.named_child(0) {
                match inner.kind() {
                    "assignment_expression" => {
                        try_emit_global_assignment(&inner, src, alias_globals, symbols);
                    }
                    "call_expression" => {
                        // Top-level IIFE — recurse into its body with the
                        // first parameter promoted to a global alias.
                        descend_iife(&inner, src, alias_globals, symbols);
                    }
                    // `(function(){…}(args))` — the UMD form where arguments
                    // are INSIDE the outer parens. The call_expression lives
                    // one level deeper than the `(function(){})(args)` form.
                    "parenthesized_expression" => {
                        if let Some(call) = inner.named_child(0) {
                            if call.kind() == "call_expression" {
                                descend_iife(&call, src, alias_globals, symbols);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // `var jQuery = window.jQuery = ...` / `var $ = jQuery` at top level —
        // the main extractor already captures these. Nothing to do here.
        _ => {}
    }
}

/// If `call` is an IIFE (`(function(){…})()` / `(()=>{})()`), walk its
/// body's top-level statements. The IIFE's first parameter name is added
/// to `alias_globals` so assignments like `root.X = …` inside register
/// globals too.
///
/// Also pairs each parameter with its call-site argument. When an argument
/// is `this` / `window` / `global` / `globalThis` / `self`, the parameter
/// joins `alias_globals`. When an argument is a string literal, it is
/// recorded so UMD-style `root[paramName] = factory()` assignments can
/// resolve the subscript and emit the literal as a global (the slugify /
/// jQuery UMD template).
fn descend_iife(
    call: &Node,
    src: &[u8],
    alias_globals: &[String],
    symbols: &mut Vec<Sym>,
) {
    let Some(func) = call.child_by_field_name("function") else { return };
    let inner_func = match func.kind() {
        "parenthesized_expression" => match func.named_child(0) {
            Some(n) => n,
            None => return,
        },
        "function_expression" | "arrow_function" => func,
        _ => return,
    };
    if !matches!(
        inner_func.kind(),
        "function_expression" | "arrow_function"
    ) {
        return;
    }
    // Collect parameter names (positional).
    let param_names: Vec<String> = inner_func
        .child_by_field_name("parameters")
        .map(|params| {
            let mut pcursor = params.walk();
            params
                .named_children(&mut pcursor)
                .map(|p| match p.kind() {
                    "identifier" => node_text(p, src),
                    "required_parameter" | "optional_parameter" | "formal_parameters" => p
                        .child_by_field_name("pattern")
                        .or_else(|| p.named_child(0))
                        .filter(|n| n.kind() == "identifier")
                        .map(|n| node_text(n, src))
                        .unwrap_or_default(),
                    _ => String::new(),
                })
                .collect()
        })
        .unwrap_or_default();

    // Collect call-site argument nodes (positional).
    let args: Vec<Node> = call
        .child_by_field_name("arguments")
        .map(|args_node| {
            let mut ac = args_node.walk();
            args_node.named_children(&mut ac).collect()
        })
        .unwrap_or_default();

    // Build the alias/binding state. Any parameter bound to a root-like
    // value becomes a global alias. Any parameter bound to a string literal
    // is recorded for UMD subscript resolution.
    let mut new_aliases: Vec<String> = alias_globals.to_vec();
    let mut string_bindings: HashMap<String, String> = HashMap::new();

    for (i, pname) in param_names.iter().enumerate() {
        if pname.is_empty() {
            continue;
        }
        // Preserve legacy behavior: the FIRST parameter always becomes a
        // global alias (the historical jQuery / Bootstrap IIFE pattern has
        // no args and relies on parameter-position convention).
        if i == 0 {
            new_aliases.push(pname.clone());
        }
        let Some(arg) = args.get(i) else { continue };
        match arg.kind() {
            "this" => {
                if !new_aliases.contains(pname) {
                    new_aliases.push(pname.clone());
                }
            }
            "identifier" => {
                let ident = node_text(*arg, src);
                if matches!(ident.as_str(), "window" | "global" | "globalThis" | "self" | "root") {
                    if !new_aliases.contains(pname) {
                        new_aliases.push(pname.clone());
                    }
                }
            }
            "string" => {
                if let Some(lit) = string_literal_value(*arg, src) {
                    string_bindings.insert(pname.clone(), lit);
                }
            }
            _ => {}
        }
    }

    let Some(body) = inner_func.child_by_field_name("body") else { return };

    // UMD subscript pass: scan for `root[name] = factory()` where `root` is
    // a global alias and `name` is a parameter bound to a string literal.
    if !string_bindings.is_empty() {
        scan_umd_subscript_exports(body, src, &new_aliases, &string_bindings, symbols);
    }

    // Existing top-level-assignment pass inside the IIFE body.
    let mut bcursor = body.walk();
    for child in body.named_children(&mut bcursor) {
        scan_statement_for_globals(child, src, &new_aliases, symbols);
    }
}

/// Walk an IIFE body and emit globals for UMD subscript assignments like
/// `root[name] = factory()` where `root` is an IIFE parameter bound to a
/// root-like value (`this`, `window`, …) and `name` is an IIFE parameter
/// bound to a string literal passed at the call site. These are the
/// canonical UMD export pattern for classic libraries (slugify, dayjs-style
/// single-file modules, etc.).
fn scan_umd_subscript_exports(
    node: Node,
    src: &[u8],
    alias_globals: &[String],
    string_bindings: &HashMap<String, String>,
    symbols: &mut Vec<Sym>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "assignment_expression" {
            try_emit_umd_subscript(&child, src, alias_globals, string_bindings, symbols);
        }
        scan_umd_subscript_exports(child, src, alias_globals, string_bindings, symbols);
    }
}

fn try_emit_umd_subscript(
    assign: &Node,
    src: &[u8],
    alias_globals: &[String],
    string_bindings: &HashMap<String, String>,
    symbols: &mut Vec<Sym>,
) {
    let Some(left) = assign.child_by_field_name("left") else { return };
    if left.kind() != "subscript_expression" {
        return;
    }
    let Some(object) = left.child_by_field_name("object") else { return };
    if object.kind() != "identifier" {
        return;
    }
    let object_name = node_text(object, src);
    if !alias_globals.iter().any(|a| a == &object_name) {
        return;
    }
    let Some(index) = left.child_by_field_name("index") else { return };
    if index.kind() != "identifier" {
        return;
    }
    let index_name = node_text(index, src);
    let Some(literal) = string_bindings.get(&index_name) else { return };
    // Emit as Function: UMD exports are almost always callable (factory()).
    push_typed_global_symbol(literal, SymbolKind::Function, assign, symbols);
}

/// Recognise `obj.X = Y` / `obj.X = obj.Y = Z` and emit every property
/// name on the LHS (including chained assignments) as a top-level global.
fn try_emit_global_assignment(
    assign: &Node,
    src: &[u8],
    alias_globals: &[String],
    symbols: &mut Vec<Sym>,
) {
    let Some(left) = assign.child_by_field_name("left") else { return };
    let Some(right) = assign.child_by_field_name("right") else { return };

    if lhs_targets_global(&left, src, alias_globals) {
        if let Some(name) = member_property_name(&left, src) {
            push_global_symbol(&name, assign, symbols);
        }
    }

    // Chained assignment: RHS may itself be an assignment_expression.
    if right.kind() == "assignment_expression" {
        try_emit_global_assignment(&right, src, alias_globals, symbols);
    }
}

/// True when the LHS of an assignment resolves to a property of a known
/// global object — directly (`window.X`), via an IIFE parameter alias
/// (`root.X` when `root` was the IIFE's first parameter), or via
/// top-level `this` (UMD wrappers in sloppy mode).
fn lhs_targets_global(lhs: &Node, src: &[u8], alias_globals: &[String]) -> bool {
    let (object, _) = match lhs.kind() {
        "member_expression" | "subscript_expression" => (
            match lhs.child_by_field_name("object") {
                Some(o) => o,
                None => return false,
            },
            lhs.kind(),
        ),
        _ => return false,
    };
    match object.kind() {
        "identifier" => {
            let name = node_text(object, src);
            matches!(name.as_str(), "window" | "global" | "globalThis" | "self" | "root")
                || alias_globals.iter().any(|a| a == &name)
        }
        "this" => true,
        _ => false,
    }
}

/// Extract the string property name from a member or subscript LHS.
fn member_property_name(lhs: &Node, src: &[u8]) -> Option<String> {
    match lhs.kind() {
        "member_expression" => lhs
            .child_by_field_name("property")
            .map(|n| node_text(n, src))
            .filter(|s| !s.is_empty()),
        "subscript_expression" => {
            // `foo["X"]` — only accept string-literal indices.
            let idx = lhs.child_by_field_name("index")?;
            if idx.kind() != "string" {
                return None;
            }
            let raw = node_text(idx, src);
            let trimmed = raw
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))?;
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    }
}

fn push_global_symbol(name: &str, anchor: &Node, symbols: &mut Vec<Sym>) {
    // Deduplicate against symbols already pushed in this file. The main
    // extractor may have captured the same name via another path (unlikely
    // for globals inside IIFE bodies, but cheap insurance).
    if symbols.iter().any(|s| s.name == name && s.parent_index.is_none()) {
        return;
    }
    let line = anchor.start_position().row as u32;
    symbols.push(Sym {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Variable,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
}
