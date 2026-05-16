// =============================================================================
// languages/cmake/arguments.rs  —  CMake argument-list parsing helpers
//
// Reads `normal_command` arguments off the tree-sitter parse tree. Provides
// raw and normalized variants; normalized form strips `${...}` / `$ENV{...}` /
// `$CACHE{...}` wrappers and skips `$<...>` generator expressions.
// =============================================================================

use super::extract::node_text;
use tree_sitter::Node;

/// Get the identifier of the command in a `normal_command` node.
pub(super) fn command_identifier(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let t = node_text(child, src);
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

/// Get the text of the first argument in a command's argument_list.
pub(super) fn first_argument_text(node: &Node, src: &str) -> Option<String> {
    nth_argument_from_node(node, src, 0)
}

/// Get the Nth argument from a command node.
pub(super) fn nth_argument(node: &Node, src: &str, n: usize) -> Option<String> {
    nth_argument_from_node(node, src, n)
}

fn nth_argument_from_node(node: &Node, src: &str, n: usize) -> Option<String> {
    // Arguments are in an `argument_list` child, or directly as `argument`/`word` children.
    let args = collect_arguments(node, src);
    args.into_iter().nth(n)
}

/// Collect raw (un-normalized) argument texts from a command node.
/// Used to detect which arguments were variable refs (`${VAR}`) vs bare names.
pub(super) fn collect_raw_arguments(node: &Node, src: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "argument_list" => {
                let mut ac = child.walk();
                for arg in child.children(&mut ac) {
                    let text = match arg.kind() {
                        "unquoted_argument" | "argument" | "identifier" | "word" => {
                            node_text(arg, src).trim().to_string()
                        }
                        "quoted_argument" => {
                            node_text(arg, src).trim_matches('"').to_string()
                        }
                        _ => String::new(),
                    };
                    if !text.is_empty() {
                        args.push(text);
                    }
                }
            }
            "argument" | "unquoted_argument" => {
                let text = node_text(child, src).trim().to_string();
                if !text.is_empty() {
                    args.push(text);
                }
            }
            "quoted_argument" => {
                let raw = node_text(child, src);
                let stripped = raw.trim_matches('"').to_string();
                if !stripped.is_empty() {
                    args.push(stripped);
                }
            }
            _ => {}
        }
    }
    args
}

/// Collect all argument texts from a command node.
/// Variable references (`${VAR}`) are stripped to bare `VAR`.
/// Generator expressions (`$<...>`) are skipped.
pub(super) fn collect_arguments(node: &Node, src: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "argument_list" => {
                let mut ac = child.walk();
                for arg in child.children(&mut ac) {
                    let text = extract_argument_text(&arg, src);
                    if !text.is_empty() {
                        args.push(text);
                    }
                }
            }
            "argument" | "unquoted_argument" => {
                let raw = node_text(child, src).trim().to_string();
                let text = normalize_argument(&raw);
                if !text.is_empty() {
                    args.push(text);
                }
            }
            "quoted_argument" => {
                // Strip surrounding quotes; quoted args with expansions are not symbol refs.
                let raw = node_text(child, src);
                let stripped = raw.trim_matches('"').to_string();
                if !stripped.is_empty() {
                    args.push(stripped);
                }
            }
            _ => {}
        }
    }
    args
}

fn extract_argument_text(node: &Node, src: &str) -> String {
    match node.kind() {
        "unquoted_argument" | "argument" | "identifier" | "word" => {
            let raw = node_text(*node, src).trim().to_string();
            normalize_argument(&raw)
        }
        "quoted_argument" => {
            let raw = node_text(*node, src);
            raw.trim_matches('"').to_string()
        }
        _ => String::new(),
    }
}

/// Normalize a raw CMake argument:
/// - `${VAR}` → `VAR`
/// - `$ENV{VAR}` → `VAR`
/// - `$CACHE{VAR}` → `VAR`
/// - `$<...>` generator expressions → empty string (caller should skip)
pub(super) fn normalize_argument(raw: &str) -> String {
    let s = raw.trim();
    // Generator expression — skip entirely
    if s.starts_with("$<") {
        return String::new();
    }
    // Variable reference — strip ${ } wrappers
    if s.starts_with("${") && s.ends_with('}') {
        return s[2..s.len() - 1].trim().to_string();
    }
    if s.starts_with("$ENV{") && s.ends_with('}') {
        return s[5..s.len() - 1].trim().to_string();
    }
    if s.starts_with("$CACHE{") && s.ends_with('}') {
        return s[7..s.len() - 1].trim().to_string();
    }
    s.to_string()
}
