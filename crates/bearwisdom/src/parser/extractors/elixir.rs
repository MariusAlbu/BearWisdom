// =============================================================================
// parser/extractors/elixir.rs  —  Elixir symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Module (→ Class), Function (def/defp), Macro (defmacro/defmacrop),
//   Struct (defstruct → Struct), Variable (module attribute bindings)
//
// REFERENCES:
//   - `alias`, `import`, `use`, `require` → Imports edges
//   - Function calls inside function bodies → Calls edges
//   - Module attributes (@moduledoc, @doc, @spec) are captured as
//     doc_comment / signature on the owning symbol where possible;
//     standalone attributes become Variable symbols.
//
// Approach:
//   Single-pass recursive CST walk.  Elixir's tree-sitter grammar represents
//   the AST as `call` nodes with `identifier` function names.  We match on
//   the callee name to dispatch to the appropriate handler.
//
// Elixir grammar node kinds (tree-sitter-elixir 0.3):
//   source, call, identifier, alias, arguments, do_block, block,
//   binary_operator, atom, string, list, unary_operator (@)
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_elixir::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Elixir grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit(tree.root_node(), source, &mut symbols, &mut refs, None, "");

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn visit(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            dispatch_call(&child, src, symbols, refs, parent_index, qualified_prefix);
        } else if child.kind() == "unary_operator" {
            // Module attributes: `@moduledoc "..."`, `@doc "..."`, `@spec name(...)`
            dispatch_attribute(&child, src, symbols, parent_index, qualified_prefix);
        } else {
            visit(child, src, symbols, refs, parent_index, qualified_prefix);
        }
    }
}

/// Dispatch on the callee name of a `call` node.
fn dispatch_call(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let callee = match call_identifier(node, src) {
        Some(c) => c,
        None => {
            // Unknown call — still visit children for nested defs
            visit(*node, src, symbols, refs, parent_index, qualified_prefix);
            return;
        }
    };

    match callee.as_str() {
        "defmodule" => extract_module(node, src, symbols, refs, parent_index, qualified_prefix),
        "def" | "defp" => extract_function(node, src, symbols, refs, parent_index, qualified_prefix, false),
        "defmacro" | "defmacrop" => extract_function(node, src, symbols, refs, parent_index, qualified_prefix, true),
        "defstruct" => extract_struct(node, src, symbols, parent_index, qualified_prefix),
        "alias" => extract_directive(node, src, symbols.len(), refs, "alias"),
        "import" => extract_directive(node, src, symbols.len(), refs, "import"),
        "use" => extract_directive(node, src, symbols.len(), refs, "use"),
        "require" => extract_directive(node, src, symbols.len(), refs, "require"),
        _ => {
            // Generic call — only record if we are inside a function
            // (parent_index being Some is a reasonable proxy for that).
            if let Some(pi) = parent_index {
                let name = callee.rsplit('.').next().unwrap_or(&callee).to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: pi,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: node.start_position().row as u32,
                    module: None,
                });
            }
            // Still recurse for nested defs / aliases inside do-block
            visit(*node, src, symbols, refs, parent_index, qualified_prefix);
        }
    }
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

fn extract_module(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    // `defmodule ModuleName do ... end`
    // Arguments node contains the module alias as first child.
    let module_name = module_name_from_call(node, src).unwrap_or_else(|| "Module".to_string());
    let qualified_name = qualify(&module_name, qualified_prefix);
    let new_prefix = qualified_name.clone();
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: module_name.clone(),
        qualified_name,
        kind: SymbolKind::Class, // modules are the top-level unit in Elixir
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("defmodule {module_name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Recurse into the do_block body
    let do_block_idx = find_do_block_index(node);
    if let Some(i) = do_block_idx {
        if let Some(do_block) = node.child(i) {
            visit(do_block, src, symbols, refs, Some(idx), &new_prefix);
        }
    }
}

// ---------------------------------------------------------------------------
// Function / Macro
// ---------------------------------------------------------------------------

fn extract_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    is_macro: bool,
) {
    // `def name(args) do ... end`  or  `def name(args), do: ...`
    let (func_name, arity) = function_name_arity(node, src);
    if func_name.is_empty() {
        return;
    }

    let sig = if arity > 0 {
        format!("{}/{}", func_name, arity)
    } else {
        func_name.clone()
    };
    let qualified_name = qualify(&func_name, qualified_prefix);

    let visibility = if is_private_def(node, src) {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: func_name,
        qualified_name,
        kind: if is_macro { SymbolKind::Function } else { SymbolKind::Method },
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(sig),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Extract calls from do_block
    let do_block_idx = find_do_block_index(node);
    if let Some(i) = do_block_idx {
        if let Some(do_block) = node.child(i) {
            extract_calls_recursive(&do_block, src, idx, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// Struct
// ---------------------------------------------------------------------------

fn extract_struct(
    node: &Node,
    _src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    // `defstruct [:field1, :field2]` — the struct name is the enclosing module.
    // We emit a Struct symbol using the enclosing module name from qualified_prefix.
    let struct_name = qualified_prefix
        .rsplit('.')
        .next()
        .unwrap_or(qualified_prefix)
        .to_string();
    if struct_name.is_empty() {
        return;
    }
    let qualified_name = qualify(&struct_name, qualified_prefix);

    symbols.push(ExtractedSymbol {
        name: struct_name.clone(),
        qualified_name,
        kind: SymbolKind::Struct,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("defstruct {struct_name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Directives: alias / import / use / require
// ---------------------------------------------------------------------------

fn extract_directive(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
    directive: &str,
) {
    // The first argument after the directive keyword is the module reference.
    // In the CST it appears in the `arguments` node.
    let target = directive_target(node, src).unwrap_or_default();
    if target.is_empty() {
        return;
    }

    // For `alias Foo.Bar, as: Baz` use the original module path as the module.
    let module = if target.contains('.') {
        Some(target.clone())
    } else {
        None
    };

    let simple = target
        .rsplit('.')
        .next()
        .unwrap_or(&target)
        .to_string();

    let _ = directive; // used for context; all become Imports edges
    refs.push(ExtractedRef {
        source_symbol_index: current_symbol_count,
        target_name: simple,
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module,
    });
}

// ---------------------------------------------------------------------------
// Module attribute dispatch  (@moduledoc / @doc / @spec / other)
// ---------------------------------------------------------------------------

fn dispatch_attribute(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    // `@name value`  — in the CST this is `unary_operator` with operator `@`
    // and a call or identifier child.
    let text = node_text(*node, src);
    // @moduledoc / @doc / @spec are documentation; surface as Variable symbols
    // so they show up as module-level metadata.
    let attr_name = attribute_name(node, src);
    if attr_name.is_empty() {
        return;
    }

    match attr_name.as_str() {
        "moduledoc" | "doc" | "spec" | "type" | "callback" => {
            // Capture as a Variable symbol so it is discoverable.
            let qualified_name = qualify(&format!("@{attr_name}"), qualified_prefix);
            symbols.push(ExtractedSymbol {
                name: format!("@{attr_name}"),
                qualified_name,
                kind: SymbolKind::Variable,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: Some(text.lines().next().unwrap_or("").trim().to_string()),
                doc_comment: None,
                scope_path: scope_from_prefix(qualified_prefix),
                parent_index,
            });
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Call extraction inside function bodies
// ---------------------------------------------------------------------------

fn extract_calls_recursive(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(callee) = call_identifier(&child, src) {
                // Skip Elixir definition keywords
                if !matches!(callee.as_str(), "def" | "defp" | "defmacro" | "defmacrop" | "defmodule" | "defstruct" | "alias" | "import" | "use" | "require") {
                    let simple = callee.rsplit('.').next().unwrap_or(&callee).to_string();
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: simple,
                        kind: EdgeKind::Calls,
                        line: child.start_position().row as u32,
                        module: None,
                    });
                }
            }
        }
        extract_calls_recursive(&child, src, source_symbol_index, refs);
    }
}

// ---------------------------------------------------------------------------
// AST helpers
// ---------------------------------------------------------------------------

/// Return the identifier/alias name that is the callee of a `call` node.
/// In Elixir's grammar the callee is typically the first child identifier.
fn call_identifier(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => return Some(node_text(child, src)),
            "alias" => return Some(node_text(child, src)),
            "dot" | "." => {}
            _ => {}
        }
        // Only inspect the first meaningful child
        if child.kind() != "comment" {
            break;
        }
    }
    // Fallback: use first non-whitespace text token
    let raw = node_text(*node, src);
    let first_word: String = raw
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if first_word.is_empty() { None } else { Some(first_word) }
}

/// Extract the module name from `defmodule ModuleName do ... end`.
fn module_name_from_call(node: &Node, src: &str) -> Option<String> {
    // Arguments node is the second child of `call` after the identifier.
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
                    // First child of arguments
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
fn function_name_arity(node: &Node, src: &str) -> (String, usize) {
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
                    // `def name(a, b)` — the inner call has name + arguments
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
                    // `def name(a, b)` at a different parse depth
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
fn is_private_def(node: &Node, src: &str) -> bool {
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
fn directive_target(node: &Node, src: &str) -> Option<String> {
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
/// The caller retrieves the node with `node.child(index)` — we return the index
/// rather than the node itself to avoid cursor lifetime issues.
fn find_do_block_index(node: &Node) -> Option<usize> {
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
fn attribute_name(node: &Node, src: &str) -> String {
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

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

/// Return the text of the first child whose kind matches `kind`.
/// Uses indexed access to avoid cursor lifetime issues.
fn first_child_text_of_kind(node: &Node, src: &str, kind: &str) -> Option<String> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == kind {
                return Some(node_text(child, src));
            }
        }
    }
    None
}

fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() { None } else { Some(prefix.to_string()) }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "elixir_tests.rs"]
mod tests;
