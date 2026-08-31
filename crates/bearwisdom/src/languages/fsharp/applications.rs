//! F# application_expression and dot_expression ref collection.
//!
//! Walks call sites in function bodies and emits `Calls` refs.

use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

use super::extract::{is_keyword, node_text};

// ---------------------------------------------------------------------------
// Collect application_expression calls and dot_expression member accesses
// ---------------------------------------------------------------------------

/// Walk the leftmost spine of nested application_expressions to find the callee name.
///
/// `f x y` → application_expression(application_expression(f, x), y)
/// The leaf callee is the first child that is NOT application_expression.
fn extract_application_callee(node: &Node, src: &str) -> String {
    let mut current = *node;
    loop {
        if let Some(first) = current.child(0) {
            match first.kind() {
                "application_expression" => {
                    current = first;
                }
                "long_identifier_or_op" | "identifier" => {
                    return node_text(&first, src).to_string();
                }
                "dot_expression" => {
                    // e.g. `obj.Method arg` — the callee is the dot member.
                    // A pure name-chain receiver keeps its full spine so the
                    // ref can carry it; a computed receiver falls back to the
                    // member alone.
                    return dot_spine_text(&first, src)
                        .or_else(|| extract_dot_member(&first, src))
                        .unwrap_or_default();
                }
                "paren_expression" | "begin_end_expression" => {
                    // e.g. `(fun x -> x) arg` — anonymous application
                    return String::new();
                }
                "infix_expression" | "ce_expression" => {
                    // e.g. `route >=> text` or `async { ... }` — compound expression,
                    // not a simple callee. The individual function refs inside will be
                    // collected by collect_applications recursing into children.
                    return String::new();
                }
                _ => {
                    // Only return text for leaf nodes (operators, keywords).
                    // Complex nodes (with children) are expressions that shouldn't
                    // be flattened into a single function name.
                    if first.child_count() == 0 {
                        let t = node_text(&first, src).to_string();
                        return t;
                    }
                    return String::new();
                }
            }
        } else {
            break;
        }
    }
    String::new()
}

pub(super) fn collect_applications(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "application_expression" => {
                // Extract the callee name: walk the leftmost spine of nested
                // application_expressions to find the actual function identifier.
                // `f x y` parses as application_expression(application_expression(f, x), y)
                // so we must recurse left to find `f`.
                let name = extract_application_callee(&child, src);
                if !name.is_empty() && !is_keyword(&name) {
                    let (target_name, chain) = split_dotted(&name, &child, true);
                    refs.push(ExtractedRef {
                        is_include: false,
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index: source_idx,
                        target_name,
                        kind: EdgeKind::Calls,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain,
                        byte_offset: child.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
            "dot_expression" => {
                // dot_expression: `expr.member` — emit a Calls ref for the member name.
                // Structure: dot_expression → [expr, ".", long_identifier_or_op | identifier]
                // A pure name-chain receiver contributes its spine to the ref's
                // chain; a computed receiver leaves only the member's own text.
                if let Some(member) = extract_dot_member(&child, src) {
                    if !member.is_empty() && !is_keyword(&member) {
                        // Chain from the full spine when it splits cleanly;
                        // otherwise the member's own text (dotted or bare).
                        let (target_name, chain) = dot_spine_text(&child, src)
                            .map(|full| split_dotted(&full, &child, false))
                            .filter(|(_, c)| c.is_some())
                            .unwrap_or_else(|| split_dotted(&member, &child, false));
                        refs.push(ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index: source_idx,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: child.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain,
                            byte_offset: child.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                        });
                    }
                }
            }
            // Union-case / module reference in PATTERN position: `| None -> …`,
            // `| Some x -> …`. The case name is the pattern's leading
            // identifier; nested binding patterns (`x` in `Some x`) fail the
            // capitalization gate below.
            "identifier_pattern" => {
                if let Some(name) = leading_identifier_text(&child, src) {
                    push_capitalized_value_ref(name, &child, source_idx, refs);
                }
            }
            // Union-case / module reference in VALUE position: `let x = None`,
            // `f x None`, `xs |> List.choose Some`. Locals are lowercase by F#
            // convention, so the capitalization gate bounds emission to
            // case/module-shaped names. A dot_expression's member side is
            // excluded — the dot handler above owns it at a different byte
            // offset; an application callee collapses in the pipeline's
            // per-site dedup (same kind + name + byte offset).
            "long_identifier_or_op" if node.kind() != "dot_expression" => {
                let t = node_text(&child, src).to_string();
                push_capitalized_value_ref(t, &child, source_idx, refs);
            }
            _ => {}
        }
        collect_applications(&child, src, source_idx, refs);
    }
}

/// Emit a `Calls` ref for a capitalized bare/dotted value or pattern
/// identifier. Lowercase names (locals, parameters) and keywords are dropped.
fn push_capitalized_value_ref(
    name: String,
    node: &Node,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let starts_upper = name.chars().next().is_some_and(|c| c.is_uppercase());
    if !starts_upper || is_keyword(&name) {
        return;
    }
    let (target_name, chain) = split_dotted(&name, node, false);
    refs.push(ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        col: 0,
        module: None,
        chain,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

/// The leading identifier of an `identifier_pattern` — the union-case or
/// active-pattern name being matched, before any nested binding pattern.
fn leading_identifier_text(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "long_identifier_or_op" | "long_identifier" | "identifier" => {
                let t = node_text(&child, src).to_string();
                if !t.is_empty() {
                    return Some(t);
                }
            }
            _ => {}
        }
    }
    None
}

/// Extract the member name from a `dot_expression` node.
///
/// Grammar: `dot_expression = expr "." long_identifier_or_op`
/// The member name is in the last `long_identifier_or_op` or `identifier` child.
fn extract_dot_member(node: &Node, src: &str) -> Option<String> {
    let mut last_ident: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "long_identifier_or_op" | "identifier" => {
                let t = node_text(&child, src).to_string();
                if !t.is_empty() {
                    last_ident = Some(t);
                }
            }
            _ => {}
        }
    }
    last_ident
}

/// The full dotted text of a `dot_expression` whose receiver spine is made
/// only of names (`obj.Method`, `Sub.Mod.value`). A computed receiver
/// (`(f x).Member`, indexers, call results) has no name spine — `None`, and
/// the caller falls back to the terminal member alone.
fn dot_spine_text(node: &Node, src: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "dot_expression" => parts.push(dot_spine_text(&child, src)?),
            "identifier" | "long_identifier" | "long_identifier_or_op" => {
                let t = node_text(&child, src);
                if t.is_empty() {
                    return None;
                }
                parts.push(t.to_string());
            }
            "." => {}
            _ => return None,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("."))
    }
}

/// Split a dotted identifier into its leaf target plus a full `MemberChain`
/// (root included, leaf last) so the generic ROOT→MEMBER walk can anchor on
/// the root and step to the leaf. A single-segment name keeps `chain: None`;
/// text with non-identifier segments (operators, ranges, backtick names) is
/// left whole so it never produces a bogus chain.
fn split_dotted(name: &str, node: &Node, leaf_is_call: bool) -> (String, Option<MemberChain>) {
    if !name.contains('.') {
        return (name.to_string(), None);
    }
    let parts: Vec<&str> = name.split('.').collect();
    if !parts.iter().all(|p| is_identifier_like(p)) {
        return (name.to_string(), None);
    }
    let node_kind = node.kind();
    let byte_offset = node.start_byte() as u32;
    let last = parts.len() - 1;
    let segments = parts
        .iter()
        .enumerate()
        .map(|(i, part)| ChainSegment {
            name: (*part).to_string(),
            node_kind: node_kind.to_string(),
            kind: segment_kind(i == 0, part),
            declared_type: None,
            type_args: Vec::new(),
            optional_chaining: false,
            byte_offset,
            declared_type_id: None,
            type_arg_ids: Vec::new(),
            is_call: leaf_is_call && i == last,
            call_args: Vec::new(),
        })
        .collect();
    (parts[last].to_string(), Some(MemberChain { segments }))
}

/// Root segment: `NamespaceAccess` for a capitalized module/type head,
/// `Identifier` for a lowercase value head. Every later segment is a
/// `Property` step.
fn segment_kind(is_root: bool, part: &str) -> SegmentKind {
    if !is_root {
        return SegmentKind::Property;
    }
    if part.chars().next().is_some_and(|c| c.is_uppercase()) {
        SegmentKind::NamespaceAccess
    } else {
        SegmentKind::Identifier
    }
}

/// True when a dotted segment is a plain F# identifier (letters, digits,
/// underscores, apostrophes; alphabetic or underscore head).
fn is_identifier_like(part: &str) -> bool {
    let mut chars = part.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
}

#[cfg(test)]
#[path = "applications_tests.rs"]
mod tests;
