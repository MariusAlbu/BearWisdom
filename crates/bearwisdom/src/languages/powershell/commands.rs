// =============================================================================
// Command (cmdlet invocation) extraction
//
// Emits a `Calls` edge for every well-formed PowerShell `command` node and
// for method invocations encountered while walking a subtree. Filters out
// extractor noise where tree-sitter-powershell labels operator fragments
// (range expressions, numeric literals) as `command` nodes.
// =============================================================================

use super::node_helpers::{find_child_text, invokation_module, node_text};
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

/// Does `name` look like a real PowerShell command/cmdlet name? Filters
/// out extractor noise where tree-sitter-powershell produces a `command`
/// node for a numeric / operator fragment (the classic offender being
/// range expressions like `0..($n - 1)` inside a `foreach`).
///
/// Real names start with an ASCII letter or `_` and contain only
/// alphanumerics plus `-`, `_`, `.`, `:`, `\`. Names with leading digits,
/// `.`, or operator punctuation are rejected.
fn looks_like_powershell_command_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '\\'))
}

pub(super) fn extract_command(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The command name is in the `command_name` child
    let cmd_name = match find_child_text(node, "command_name", src) {
        Some(n) => n,
        None => return,
    };

    // Filter garbage: tree-sitter-powershell occasionally parses expressions
    // like `0..($n - 1)` (range operator) or operator fragments as `command`
    // with a synthetic `command_name`. A real PowerShell command name starts
    // with an ASCII letter or `_`, and contains only letters / digits / `-`
    // / `_` / `.` / `:` / `\`. Anything else is extractor noise — skip so it
    // doesn't leak into `unresolved_refs`.
    if !looks_like_powershell_command_name(&cmd_name) {
        return;
    }

    // For `Import-Module`, try to extract the module name as an Imports edge;
    // fall back to emitting a Calls edge so the command node is always covered.
    if cmd_name.eq_ignore_ascii_case("Import-Module") {
        let mut emitted = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let text = node_text(&child, src);
            if child.kind() == "command_elements"
                || child.kind() == "string_literal"
                || child.kind() == "bare_string_literal"
            {
                let module = text.trim_matches(|c| c == '"' || c == '\'').to_string();
                if !module.is_empty() && module != cmd_name {
                    refs.push(ExtractedRef {
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index,
                        target_name: module.clone(),
                        kind: EdgeKind::Imports,
                        line: node.start_position().row as u32,
                        col: 0,
                        module: Some(module),
                        chain: None,
                        byte_offset: node.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                    emitted = true;
                    break;
                }
            }
        }
        if !emitted {
            // Couldn't resolve module name — still emit so the node is covered
            refs.push(ExtractedRef {
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index,
                target_name: cmd_name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                col: 0,
                module: None,
                chain: None,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }
        return;
    }

    refs.push(ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index,
        target_name: cmd_name,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        col: 0,
        module: None,
        chain: None,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

// ---------------------------------------------------------------------------
// Walk subtree collecting command/call nodes
// ---------------------------------------------------------------------------

pub(super) fn visit_for_calls(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "command" {
            extract_command(&child, src, source_idx, refs);
            // Recurse so script-block args (ForEach-Object { ... }) are visited.
            visit_for_calls(&child, src, source_idx, refs);
        } else if child.kind() == "invokation_expression" {
            // Method call: extract method name with fallbacks
            let name = find_child_text(&child, "member_name", src)
                .or_else(|| find_child_text(&child, "type_name", src))
                .or_else(|| find_child_text(&child, "simple_name", src))
                .unwrap_or_else(|| {
                    (0..child.child_count())
                        .filter_map(|i| child.child(i))
                        .find(|c| c.is_named())
                        .map(|c| node_text(&c, src).to_string())
                        .unwrap_or_default()
                });
            if !name.is_empty() {
                let module = invokation_module(&child, src);
                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: source_idx,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    col: 0,
                    module,
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
            visit_for_calls(&child, src, source_idx, refs);
        } else {
            visit_for_calls(&child, src, source_idx, refs);
        }
    }
}
