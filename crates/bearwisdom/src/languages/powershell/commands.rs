// =============================================================================
// Command (cmdlet invocation) extraction
//
// Emits a `Calls` edge for every well-formed PowerShell `command` node and
// for method invocations encountered while walking a subtree. Filters out
// extractor noise where tree-sitter-powershell labels operator fragments
// (range expressions, numeric literals) as `command` nodes.
// =============================================================================

use super::node_helpers::{find_child, find_child_text, invokation_module, node_text};
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
    // The command name is in the `command_name` child. Dot-sourcing (`. path`)
    // and the call operator (`& path`) use a different field shape entirely —
    // `command_name_expr` wrapping the target expression — handled separately
    // below so those nodes aren't silently dropped.
    let cmd_name = match find_child_text(node, "command_name", src) {
        Some(n) => n,
        None => {
            extract_invokation_operator_command(node, src, source_symbol_index, refs);
            return;
        }
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
        let module = find_child(node, "command_elements")
            .and_then(|elements| static_module_name(&elements, src, &cmd_name));
        match module {
            Some(module) => {
                refs.push(ExtractedRef {
                    is_include: false,
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
            }
            None => {
                // Module name isn't a statically known string (a variable, a
                // computed path, an interpolated string, ...) — still emit
                // so the node is covered.
                refs.push(ExtractedRef {
                    is_include: false,
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
        }
        return;
    }

    refs.push(ExtractedRef {
        is_include: false,
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

/// Find the value bound to `-Name` inside an `Import-Module` command's
/// `command_elements`, or the first positional value when no `-Name` flag is
/// present (Import-Module's first positional parameter is `-Name`). Returns
/// `None` when the candidate value isn't a statically known string, or when
/// nothing resolved to `cmd_name` itself.
///
/// `command_elements` is a flat run of `command_argument_sep`,
/// `command_parameter` (a `-Flag` token), and value nodes — a `command_parameter`
/// binds only the very next value node; two `command_parameter`s in a row
/// means the first was a switch (`-Force`) with no value of its own.
fn static_module_name(command_elements: &Node, src: &str, cmd_name: &str) -> Option<String> {
    let mut cursor = command_elements.walk();
    let mut pending_flag: Option<String> = None;
    let mut seen_positional = false;
    for child in command_elements.children(&mut cursor) {
        match child.kind() {
            "command_argument_sep" => continue,
            "command_parameter" => {
                pending_flag = Some(node_text(&child, src).to_ascii_lowercase());
            }
            _ => {
                let binds_name = match pending_flag.take() {
                    Some(flag) => flag.trim_start_matches('-') == "name",
                    None => !std::mem::replace(&mut seen_positional, true),
                };
                if binds_name {
                    let module = extract_static_string_value(&child, src)?;
                    return (!module.is_empty() && module != cmd_name).then_some(module);
                }
            }
        }
    }
    None
}

/// Unwrap a command-argument value node down to a statically known string,
/// or `None` when the value is a variable, a computed expression, or a
/// malformed/error subtree (a variable-prefixed path like `$PSScriptRoot\..`
/// parses with embedded `ERROR` nodes in tree-sitter-powershell today).
fn extract_static_string_value(node: &Node, src: &str) -> Option<String> {
    match node.kind() {
        "array_literal_expression" | "unary_expression" => {
            let mut cursor = node.walk();
            let child = node.children(&mut cursor).find(|c| c.is_named())?;
            extract_static_string_value(&child, src)
        }
        "string_literal" => {
            let text = node_text(node, src);
            Some(text.trim_matches(|c| c == '"' || c == '\'').to_string())
        }
        "generic_token" | "bare_string_literal" | "simple_name" => {
            Some(node_text(node, src).to_string())
        }
        _ => None,
    }
}

/// Handle `command` nodes shaped by the dot-source (`.`) or call (`&`)
/// operator, which the grammar gives a `command_name_expr` field instead of
/// the plain `command_name` token — `extract_command`'s normal lookup misses
/// these entirely, so without this the node produces zero refs.
///
/// Only dot-sourcing is emitted as an `Imports` edge: it loads the target
/// script's declarations into the caller's scope, matching `Import-Module`'s
/// semantics. The call operator (`&`) runs the target in a child scope and
/// doesn't bring anything into scope, so it's left uninstrumented rather than
/// mis-tagged as an import.
fn extract_invokation_operator_command(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let Some(operator) = find_child(node, "command_invokation_operator") else {
        return;
    };
    if node_text(&operator, src) != "." {
        return;
    }
    let Some(target) = find_child(node, "command_name_expr") else {
        return;
    };
    let path = node_text(&target, src).trim().to_string();
    if path.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index,
        target_name: path.clone(),
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        col: 0,
        module: Some(path),
        chain: None,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

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
                    is_include: false,
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
