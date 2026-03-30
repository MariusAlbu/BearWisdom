// =============================================================================
// elixir/helpers.rs  —  AST helpers for the Elixir extractor
// =============================================================================

use tree_sitter::Node;

pub(super) fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

pub(super) fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

pub(super) fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() { None } else { Some(prefix.to_string()) }
}

/// Return the identifier/alias name that is the callee of a `call` node.
pub(super) fn call_identifier(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => return Some(node_text(child, src)),
            "alias" => return Some(node_text(child, src)),
            "dot" | "." => {}
            _ => {}
        }
        if child.kind() != "comment" {
            break;
        }
    }
    let raw = node_text(*node, src);
    let first_word: String = raw
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if first_word.is_empty() { None } else { Some(first_word) }
}

/// Extract the module name from `defmodule ModuleName do ... end`.
pub(super) fn module_name_from_call(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    let mut found_defmodule = false;
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let t = node_text(child, src);
            if t == "defmodule" {
                found_defmodule = true;
                continue;
            }
        }
        if found_defmodule {
            match child.kind() {
                "alias" | "identifier" => return Some(node_text(child, src)),
                "arguments" => {
                    let mut ac = child.walk();
                    for arg in child.children(&mut ac) {
                        match arg.kind() {
                            "alias" | "identifier" => return Some(node_text(arg, src)),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Extract the `(name, arity)` from a `def name(a, b) do...` call node.
pub(super) fn function_name_arity(node: &Node, src: &str) -> (String, usize) {
    let mut cursor = node.walk();
    let mut past_def = false;
    for child in node.children(&mut cursor) {
        if !past_def {
            if child.kind() == "identifier" {
                let t = node_text(child, src);
                if matches!(t.as_str(), "def" | "defp" | "defmacro" | "defmacrop") {
                    past_def = true;
                    continue;
                }
            }
        } else {
            match child.kind() {
                "identifier" => {
                    return (node_text(child, src), 0);
                }
                "call" => {
                    let name_text = child
                        .child_by_field_name("name")
                        .map(|n| node_text(n, src))
                        .or_else(|| first_child_text_of_kind(&child, src, "identifier"));
                    if let Some(name) = name_text {
                        let arity = child.child_by_field_name("arguments")
                            .map(|args| {
                                let mut ac = args.walk();
                                args.children(&mut ac)
                                    .filter(|n| n.kind() != "," && n.kind() != "(" && n.kind() != ")")
                                    .count()
                            })
                            .unwrap_or(0);
                        return (name, arity);
                    }
                }
                "arguments" => {
                    let mut ac = child.walk();
                    for arg in child.children(&mut ac) {
                        if arg.kind() == "identifier" {
                            return (node_text(arg, src), 0);
                        }
                        if arg.kind() == "call" {
                            let nn_text = arg.child_by_field_name("name")
                                .map(|n| node_text(n, src))
                                .or_else(|| first_child_text_of_kind(&arg, src, "identifier"));
                            if let Some(name) = nn_text {
                                return (name, 0);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    (String::new(), 0)
}

/// True if the def keyword is `defp` or `defmacrop`.
pub(super) fn is_private_def(node: &Node, src: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let t = node_text(child, src);
            if t == "defp" || t == "defmacrop" {
                return true;
            }
            break;
        }
    }
    false
}

/// Return the module/atom target of an alias/import/use/require directive.
pub(super) fn directive_target(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    let mut past_keyword = false;
    for child in node.children(&mut cursor) {
        if !past_keyword {
            if child.kind() == "identifier" {
                past_keyword = true;
                continue;
            }
        } else {
            match child.kind() {
                "alias" | "identifier" => return Some(node_text(child, src)),
                "arguments" => {
                    let mut ac = child.walk();
                    for arg in child.children(&mut ac) {
                        match arg.kind() {
                            "alias" | "identifier" => return Some(node_text(arg, src)),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Return the child index of the do_block / block / keyword list in a `call` node.
pub(super) fn find_do_block_index(node: &Node) -> Option<usize> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            let k = child.kind();
            if k == "do_block" || k == "block" || k == "keywords" || k == "keyword_list" {
                return Some(i);
            }
        }
    }
    None
}

/// Extract the attribute name from a `@name value` unary_operator node.
pub(super) fn attribute_name(node: &Node, src: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => return node_text(child, src),
            "call" => {
                if let Some(id) = call_identifier(&child, src) {
                    return id;
                }
            }
            _ => {}
        }
    }
    String::new()
}

/// Return the text of the first child whose kind matches `kind`.
pub(super) fn first_child_text_of_kind(node: &Node, src: &str, kind: &str) -> Option<String> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == kind {
                return Some(node_text(child, src));
            }
        }
    }
    None
}
