// =============================================================================
// languages/typescript/type_scan.rs — post-traversal type-identifier scan + scope walks
// =============================================================================

use super::helpers;
use crate::types::{EdgeKind, ExtractedRef};

pub(super) fn is_ts_primitive(name: &str) -> bool {
    matches!(
        name,
        "string"
            | "number"
            | "boolean"
            | "void"
            | "any"
            | "unknown"
            | "never"
            | "undefined"
            | "null"
            | "object"
            | "symbol"
            | "bigint"
    )
}

/// Recursively scan ALL descendants of `node` for ref-producing node kinds that
/// may have been missed by the main walker due to nesting depth or expression
/// contexts not covered by a dedicated arm.
///
/// This post-traversal pass ensures:
/// - Every `type_identifier` (non-primitive) produces a TypeRef
/// - Every `type_annotation` produces a TypeRef for its enclosed type
/// - Every `as_expression` produces a TypeRef for the cast type
/// - Every `satisfies_expression` produces a TypeRef for the checked type
///
/// All refs are attributed to `sym_idx` (symbol 0 for the file-level pass).
/// The coverage metric only needs a ref at the correct line — sym_idx is not
/// checked by the correlation logic.
/// True when every leaf named child of `node` (walking nested
/// union_type / intersection_type wrappers) is a `literal_type`.
///
/// Longer literal unions (`'a' | 'b' | 'c' | 'd' | 'e'`) parse as a
/// LEFT-NESTED chain of union_types — the outer union's children are
/// not all literal_type directly, they include an inner union_type
/// that itself contains literals. A flat `children.all(literal_type)`
/// check would miss this shape and leak the first literal's content
/// as a coverage TypeRef.
/// Walk a union_type / intersection_type / parenthesized_type and return the
/// text of the first non-`literal_type` named child. Skips union/intersection
/// wrappers recursively. Returns `None` when every member is a literal_type
/// (caller should treat that as `_primitive`).
fn first_non_literal_descendant_text(node: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "literal_type" => continue,
            // `'a'[]` and `('a' | 'b')` and `Readonly<'a' | 'b'>` are all
            // structurally pure-literal containers — recurse so we don't
            // grab their literal contents as a TypeRef target.
            "union_type" | "intersection_type" | "parenthesized_type" | "array_type"
            | "readonly_type" | "tuple_type" => {
                if let Some(t) = first_non_literal_descendant_text(child, src) {
                    return Some(t);
                }
            }
            // Reference-bearing leaves — the first real type name in the composite.
            "type_identifier" | "identifier" | "generic_type" | "nested_type_identifier"
            | "member_expression" => return Some(helpers::node_text(child, src)),
            // Structured / non-reference members (object_type, function_type,
            // mapped_type, conditional_type, predefined_type, template_literal_type,
            // …) — their first token is a property / parameter name, not a type
            // ref. Skip so the caller falls back to the `_primitive` sentinel.
            _ => continue,
        }
    }
    None
}

/// The value-space operand of a `typeof X` type query — the referenced name,
/// not the `typeof` keyword. `typeof movies` → `movies`. Prefers the `name`
/// field, falling back to the first non-`typeof` child. Returns `None` for
/// `typeof import('m')` (a module-namespace query the main annotation path
/// emits as a module ref) and for empty operands.
fn type_query_operand_text(tn: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let operand = tn.child_by_field_name("name").or_else(|| {
        let mut cursor = tn.walk();
        let found = tn.children(&mut cursor).find(|c| c.kind() != "typeof");
        found
    })?;
    let text = helpers::node_text(operand, src);
    if text.is_empty() || text.starts_with("import") {
        return None;
    }
    Some(text)
}

/// The coverage-ref target for a type node `tn`: the real root type name for
/// reference-bearing forms, or the `_primitive` sentinel for forms whose first
/// identifier is NOT a type reference — primitives, literal types, and
/// structured types (object/function/mapped/conditional) whose first token is a
/// property or parameter name. The sentinel satisfies the coverage budget (a ref
/// at the node's line) without leaking `wow`/`true`/`1`/`children` into
/// unresolved_refs; `pipeline.rs` drops `_primitive` before resolution.
///
/// Shared by the `type_annotation`, `as_expression`, and `satisfies_expression`
/// arms so all three classify cast / annotation / check types identically.
fn coverage_target_for_type_node(tn: tree_sitter::Node, src: &[u8]) -> String {
    // Split a raw type-name string to its first identifier-shaped token, or the
    // `_primitive` sentinel when that token is numeric (leftover literal content).
    let root_of = |s: &str| -> String {
        let candidate = s
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .find(|t| !t.is_empty())
            .unwrap_or("_")
            .to_string();
        if candidate.is_empty() || candidate.chars().all(|c| c.is_ascii_digit()) {
            "_primitive".to_string()
        } else {
            candidate
        }
    };
    let name = helpers::node_text(tn, src);
    let target = match tn.kind() {
        // Qualified forms → emit the full dotted name so downstream
        // classification (primitives list, React namespace check) can match.
        "nested_type_identifier" | "member_expression" => name,
        // Reference-bearing leaf — its own text is the type name.
        "type_identifier" | "identifier" | "generic_type" => root_of(&name),
        // `typeof X` — the value-space operand is the ref, NOT the `typeof`
        // keyword. `root_of("typeof movies")` would grab `typeof`; extract the
        // operand instead (`movies`), mirroring the main annotation path.
        "type_query" => type_query_operand_text(tn, src)
            .map(|o| root_of(&o))
            .unwrap_or_else(|| "_primitive".to_string()),
        // Literal type by itself (`true`, `42`, `'x'`) — never a real ref.
        "literal_type" => "_primitive".to_string(),
        // Composites: a real type name only if some member is reference-bearing
        // (`'400' | Foo` → `Foo`, `User[]` → `User`). A composite of only
        // literals / structured members (`{ wow: boolean } | undefined`,
        // `'a' | 'b'`) carries no ref → sentinel.
        "union_type" | "intersection_type" | "array_type" | "tuple_type"
        | "parenthesized_type" | "readonly_type" => first_non_literal_descendant_text(tn, src)
            .map(|t| root_of(&t))
            .unwrap_or_else(|| "_primitive".to_string()),
        // Structured types — the first identifier in the text is a property
        // or parameter name (object/function/mapped/conditional), not a ref.
        _ => "_primitive".to_string(),
    };
    if target.is_empty() || is_ts_primitive(&target) {
        "_primitive".to_string()
    } else {
        target
    }
}

pub(super) fn scan_all_type_identifiers(
    node: tree_sitter::Node,
    src: &[u8],
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" if child.is_named() => {
                let name = helpers::node_text(child, src);
                if !name.is_empty() && !is_ts_primitive(&name) {
                    refs.push(ExtractedRef {
                        is_include: false,
                        is_import_binding: false,
                        is_reexport: false,
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
                // type_identifier is a leaf — no children to recurse into.
            }
            "nested_type_identifier" if child.is_named() => {
                // Qualified type like `React.ReactNode` or `Stripe.Event` — emit
                // the full dotted name as a single ref. Do NOT recurse into
                // children; the tree has a leaf type_identifier inside that
                // would otherwise be emitted as a duplicate bare ref.
                let name = helpers::node_text(child, src);
                if !name.is_empty() && !is_ts_primitive(&name) {
                    refs.push(ExtractedRef {
                        is_include: false,
                        is_import_binding: false,
                        is_reexport: false,
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
            "generic_type" if child.is_named() => {
                // Extract the base type name from the generic, e.g. `Promise<User>` → `Promise`.
                // The first named child of generic_type is the base type_identifier.
                let base_opt = child.child_by_field_name("name").or_else(|| {
                    let children: Vec<_> = {
                        let mut gc = child.walk();
                        child.children(&mut gc).collect()
                    };
                    children.into_iter().find(|c| c.kind() == "type_identifier")
                });
                if let Some(base) = base_opt {
                    let name = helpers::node_text(base, src);
                    if !name.is_empty() && !is_ts_primitive(&name) {
                        refs.push(ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index: sym_idx,
                            target_name: name,
                            kind: EdgeKind::TypeRef,
                            line: base.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain: None,
                            byte_offset: base.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                        });
                    }
                }
                // Still recurse so type arguments inside are also scanned.
                scan_all_type_identifiers(child, src, sym_idx, refs);
            }
            // Emit a TypeRef for the line of every type_annotation node found in the tree.
            // The annotation text is not important for the coverage metric — the line match
            // is what counts. We also recurse to catch nested annotations and type_identifiers.
            "type_annotation" if child.is_named() => {
                // Find the actual type node inside the annotation (after the colon).
                let type_node = {
                    let mut found = None;
                    let mut ac = child.walk();
                    for ann_child in child.children(&mut ac) {
                        if ann_child.kind() != ":" {
                            found = Some(ann_child);
                            break;
                        }
                    }
                    found
                };
                if let Some(tn) = type_node {
                    // Emit a coverage ref at the annotation line. The target is a
                    // real root type name, or the `_primitive` sentinel for
                    // primitives / literals / structured types whose first token
                    // is a property or parameter name (not a type reference).
                    let name = helpers::node_text(tn, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index: sym_idx,
                            target_name: coverage_target_for_type_node(tn, src),
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
                // Recurse to catch nested annotations and other type_identifiers,
                // but skip when the inner type is already a qualified form — the
                // recursion would otherwise leak the bare last segment.
                let skip_recurse = type_node.is_some_and(|tn| {
                    matches!(tn.kind(), "nested_type_identifier" | "member_expression")
                });
                if !skip_recurse {
                    scan_all_type_identifiers(child, src, sym_idx, refs);
                }
            }
            // Emit a TypeRef for the line of every as_expression node found in the tree.
            "as_expression" if child.is_named() => {
                // Find the type after the `as` keyword.
                let mut after_as = false;
                let mut ac = child.walk();
                for as_child in child.children(&mut ac) {
                    if as_child.kind() == "as" {
                        after_as = true;
                        continue;
                    }
                    if after_as {
                        // `x as { wow: boolean }` / `x as true` / `x as () => 1`
                        // must NOT leak the property name / literal — classify the
                        // cast type like an annotation (object/literal/function →
                        // `_primitive`, real type → its root name).
                        refs.push(ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index: sym_idx,
                            target_name: coverage_target_for_type_node(as_child, src),
                            kind: EdgeKind::TypeRef,
                            line: child.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain: None,
                            byte_offset: child.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                        });
                        break;
                    }
                }
                scan_all_type_identifiers(child, src, sym_idx, refs);
            }
            // Emit a TypeRef for the line of every satisfies_expression node found in the tree.
            "satisfies_expression" if child.is_named() => {
                // Find the type after the `satisfies` keyword.
                let mut after_satisfies = false;
                let mut sc = child.walk();
                for sat_child in child.children(&mut sc) {
                    if sat_child.kind() == "satisfies" {
                        after_satisfies = true;
                        continue;
                    }
                    if after_satisfies {
                        // Classify the checked type like an annotation so object /
                        // literal / function `satisfies` targets emit the
                        // `_primitive` sentinel instead of leaking a property name.
                        refs.push(ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index: sym_idx,
                            target_name: coverage_target_for_type_node(sat_child, src),
                            kind: EdgeKind::TypeRef,
                            line: child.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain: None,
                            byte_offset: child.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                        });
                        break;
                    }
                }
                scan_all_type_identifiers(child, src, sym_idx, refs);
            }
            _ => {
                scan_all_type_identifiers(child, src, sym_idx, refs);
            }
        }
    }
}

/// Return every type-parameter name declared on `node` via a direct
/// `type_parameters` child. Covers function / arrow / method / class /
/// interface / type-alias / call-signature / construct-signature
/// declarations — all the TS grammar nodes that can introduce a fresh
/// generic scope.
///
/// Empty vec for nodes that don't declare type parameters (most of them).
fn collect_declared_type_params(node: &tree_sitter::Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "type_parameters" {
            continue;
        }
        let mut tp_cursor = child.walk();
        for tp in child.children(&mut tp_cursor) {
            if tp.kind() != "type_parameter" {
                continue;
            }
            // Prefer the `name` field when the grammar exposes it; fall back
            // to the first type_identifier / identifier child for grammars
            // that don't name the field. The explicit loop keeps the walker
            // cursor's lifetime inside the same scope as the yielded node.
            let name_node = if let Some(n) = tp.child_by_field_name("name") {
                Some(n)
            } else {
                let mut tpc = tp.walk();
                let mut found = None;
                for c in tp.children(&mut tpc) {
                    if matches!(c.kind(), "type_identifier" | "identifier") {
                        found = Some(c);
                        break;
                    }
                }
                found
            };
            if let Some(n) = name_node {
                if let Ok(name) = n.utf8_text(src) {
                    if !name.is_empty() {
                        out.push(name.to_string());
                    }
                }
            }
        }
    }
    out
}

/// Walk the full tree and collect every `(type_param_name, start_line,
/// end_line)` tuple for the subtree where that type parameter is in scope.
///
/// Used by the post-filter pass in `extract_inner` to drop TypeRef entries
/// whose target is a generic type parameter binding (e.g. `Target` inside
/// `TargetedEvent<Target>`) rather than an external type. The scope is
/// approximated by the line range of the declaring node, which is accurate
/// enough in practice — same-line cross-scope collisions require contrived
/// single-line layouts of separate generic declarations.
pub(super) fn collect_type_param_scopes(
    node: tree_sitter::Node,
    src: &[u8],
    out: &mut Vec<(String, u32, u32)>,
) {
    let declared = collect_declared_type_params(&node, src);
    if !declared.is_empty() {
        let start_line = node.start_position().row as u32;
        let end_line = node.end_position().row as u32;
        for name in declared {
            out.push((name, start_line, end_line));
        }
    }
    // `[K in keyof T]: V[K]` mapped types bind `K` for the body of the
    // mapped type. tree-sitter-typescript represents the `[K in keyof T]`
    // header as a `mapped_type_clause` node with the binding in its
    // `name` field. Without this, `TRecord[TKey]` in
    // `{ [TKey in keyof TRecord]: TRecord[TKey] }` leaks `TKey` as
    // an external TypeRef (the `indexed_access_type` walker treats
    // the index as a regular type ref).
    //
    // Scope is the parent `mapped_type` (whose line range covers both
    // the clause AND the body). Falls back to the clause itself if
    // somehow detached.
    if node.kind() == "mapped_type_clause" {
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Ok(name) = name_node.utf8_text(src) {
                if !name.is_empty() {
                    // The binder is in scope for the whole mapped type, whose body
                    // (`AKey extends … ? T[AKey] : U[AKey]`) spans lines below the
                    // clause. The immediate parent is not always `mapped_type`, so
                    // walk up to the enclosing mapped_type / object_type to cover
                    // the body; falling back to the clause leaks body uses of the
                    // binder on later lines.
                    let scope_node = enclosing_of_kind(&node, &["mapped_type", "object_type"])
                        .unwrap_or(node);
                    out.push((
                        name.to_string(),
                        scope_node.start_position().row as u32,
                        scope_node.end_position().row as u32,
                    ));
                }
            }
        }
    }
    // `infer X` introduces a type variable inside a `conditional_type` —
    // `T extends Foo<infer X> ? X : never`. The variable is in scope for
    // the entire conditional expression. Without this, real source code
    // patterns like `T extends [infer Head, ...infer Tails]` leak `Head`
    // and `Tails` as unresolved external TypeRefs (very common in
    // tanstack-query / trpc / solid-query type gymnastics).
    if node.kind() == "infer_type" {
        let mut name_node = node.child_by_field_name("name");
        if name_node.is_none() {
            let mut cursor = node.walk();
            for c in node.children(&mut cursor) {
                if matches!(c.kind(), "type_identifier" | "identifier") {
                    name_node = Some(c);
                    break;
                }
            }
        }
        if let Some(n) = name_node {
            if let Ok(name) = n.utf8_text(src) {
                if !name.is_empty() {
                    let scope_node = enclosing_conditional_type(&node).unwrap_or(node);
                    out.push((
                        name.to_string(),
                        scope_node.start_position().row as u32,
                        scope_node.end_position().row as u32,
                    ));
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_type_param_scopes(child, src, out);
    }
}

/// Walk parents until we find the enclosing `conditional_type` node — the
/// scope an `infer X` binding is visible in. Falls back to `None` if the
/// `infer_type` is somehow detached from a conditional context (which the
/// TS grammar shouldn't produce, but defensive coding doesn't hurt).
fn enclosing_conditional_type<'a>(node: &tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    enclosing_of_kind(node, &["conditional_type"])
}

/// Walk parents until one whose kind is in `kinds`. Used to find the scope an
/// in-place type binder (mapped-type key, `infer X`) is visible in.
fn enclosing_of_kind<'a>(
    node: &tree_sitter::Node<'a>,
    kinds: &[&str],
) -> Option<tree_sitter::Node<'a>> {
    let mut cur = node.parent();
    while let Some(p) = cur {
        if kinds.contains(&p.kind()) {
            return Some(p);
        }
        cur = p.parent();
    }
    None
}
