// =============================================================================
// parser/extractors/elixir/mod.rs  —  Elixir symbol and reference extractor
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

mod helpers;

use helpers::{
    attribute_name, call_identifier, directive_target, find_do_block_index,
    function_name_arity, is_private_def, module_name_from_call, node_text, qualify,
    scope_from_prefix,
};

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
                    chain: None,
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
    let module_name = module_name_from_call(node, src).unwrap_or_else(|| "Module".to_string());
    let qualified_name = qualify(&module_name, qualified_prefix);
    let new_prefix = qualified_name.clone();
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: module_name.clone(),
        qualified_name,
        kind: SymbolKind::Class,
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
    let target = directive_target(node, src).unwrap_or_default();
    if target.is_empty() {
        return;
    }

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

    let _ = directive;
    refs.push(ExtractedRef {
        source_symbol_index: current_symbol_count,
        target_name: simple,
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module,
        chain: None,
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
    let text = node_text(*node, src);
    let attr_name = attribute_name(node, src);
    if attr_name.is_empty() {
        return;
    }

    match attr_name.as_str() {
        "moduledoc" | "doc" | "spec" | "type" | "callback" => {
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
                if !matches!(callee.as_str(), "def" | "defp" | "defmacro" | "defmacrop" | "defmodule" | "defstruct" | "alias" | "import" | "use" | "require") {
                    let simple = callee.rsplit('.').next().unwrap_or(&callee).to_string();
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: simple,
                        kind: EdgeKind::Calls,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
        }
        extract_calls_recursive(&child, src, source_symbol_index, refs);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
