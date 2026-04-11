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


use super::helpers::{
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

    let root = tree.root_node();
    visit(root, source, &mut symbols, &mut refs, None, "");

    // Post-traversal: scan the entire CST for `alias` nodes (module references
    // like `Enum`, `MyApp.User`) and `dot` nodes (module.function calls) that
    // the top-down walker may have missed. Emits TypeRef for each.
    scan_all_type_refs(root, source, &mut refs);

    // Phoenix route helper synthesis (Phase 1.2b). When the source module
    // uses `Phoenix.Router` directly OR via the Phoenix 1.5+ indirection
    // pattern `use MyAppWeb, :router` (where MyAppWeb is the project's
    // wrapper module), every declared route produces compile-time helper
    // functions `Routes.*_path` / `Routes.*_url`. BearWisdom doesn't
    // execute Elixir macros, so these names never appear as source-
    // defined symbols. Synthesise a Function symbol per derived helper
    // so the resolver can match them.
    if is_phoenix_router_module(source) {
        super::phoenix_routes::synthesize_route_helpers(source, &mut symbols);
    }

    super::ExtractionResult::new(symbols, refs, has_errors)
}

/// Detect whether an Elixir source file is a Phoenix router module that
/// should receive compile-time route helper synthesis.
///
/// Matches:
///   * `use Phoenix.Router`               (legacy direct form)
///   * `use <MyAppWeb>, :router`          (Phoenix 1.5+ indirect form — the
///                                        project's Web module re-exports
///                                        Phoenix.Router via `quote`)
fn is_phoenix_router_module(source: &str) -> bool {
    if source.contains("Phoenix.Router") {
        return true;
    }
    // Cheap substring check for the indirect form. Avoids pulling in the
    // regex crate for a simple pattern we can recognise with string ops.
    for line in source.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("use ") {
            continue;
        }
        if trimmed.contains(", :router") || trimmed.contains(",:router") {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod phoenix_router_detect_tests {
    use super::is_phoenix_router_module;

    #[test]
    fn detects_direct_phoenix_router() {
        assert!(is_phoenix_router_module(
            "defmodule Router do\n  use Phoenix.Router\nend"
        ));
    }

    #[test]
    fn detects_phoenix_15_indirect_form() {
        assert!(is_phoenix_router_module(
            "defmodule ChangelogWeb.Router do\n  use ChangelogWeb, :router\nend"
        ));
    }

    #[test]
    fn rejects_non_router_module() {
        assert!(!is_phoenix_router_module(
            "defmodule Foo do\n  def bar, do: :ok\nend"
        ));
    }

    #[test]
    fn rejects_router_alias_without_use() {
        // The module uses `:router` as a key in a struct, not a `use` macro.
        assert!(!is_phoenix_router_module(
            "defmodule Foo do\n  @opts [type: :router]\nend"
        ));
    }
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
            dispatch_attribute(&child, src, symbols, refs, parent_index, qualified_prefix);
        } else if child.kind() == "binary_operator" {
            // Pipe operators and other binary expressions at module scope.
            let sym_idx = parent_index.unwrap_or(0);
            extract_pipe_calls(&child, src, sym_idx, refs);
            visit(child, src, symbols, refs, parent_index, qualified_prefix);
        } else if child.kind() == "alias" {
            // Module reference in module-level expression (e.g., `MyApp.Repo` in attributes).
            let sym_idx = parent_index.unwrap_or(0);
            let name = node_text(child, src);
            if !name.is_empty() {
                let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: simple,
                    kind: EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    module: if name.contains('.') { Some(name) } else { None },
                    chain: None,
                });
            }
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
        "defexception" => extract_exception(node, src, symbols, parent_index, qualified_prefix),
        "defprotocol" => extract_protocol(node, src, symbols, refs, parent_index, qualified_prefix),
        "defimpl" => extract_implementation(node, src, symbols, refs, parent_index, qualified_prefix),
        "defguard" | "defguardp" => extract_function(node, src, symbols, refs, parent_index, qualified_prefix, false),
        "alias" => extract_directive(node, src, symbols.len(), refs, "alias"),
        "import" => extract_directive(node, src, symbols.len(), refs, "import"),
        "use" => extract_directive(node, src, symbols.len(), refs, "use"),
        "require" => extract_directive(node, src, symbols.len(), refs, "require"),
        _ => {
            // Generic call — emit Calls edge from the enclosing symbol (or symbol 0
            // as a fallback for module-level calls that have no enclosing function).
            let sym_idx = parent_index.unwrap_or(0);
            let name = callee.rsplit('.').next().unwrap_or(&callee).to_string();
            refs.push(ExtractedRef {
                source_symbol_index: sym_idx,
                target_name: name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
            });
            // For dot calls (e.g. `Enum.map`), also emit a TypeRef for the receiver module.
            extract_dot_call_module_ref(node, src, sym_idx, refs);

            // Extract calls from nested function blocks (e.g., inside Enum.map's fn...end).
            // This ensures we capture all function calls within blocks and arguments.
            let do_block_idx = find_do_block_index(node);
            if let Some(i) = do_block_idx {
                if let Some(do_block) = node.child(i) {
                    extract_calls_recursive(&do_block, src, sym_idx, refs);
                }
            }

            // Still recurse for nested defs / aliases inside arguments.
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
// Exception (defexception → Struct)
// ---------------------------------------------------------------------------

/// `defexception [:message, ...]` — emits a `Struct` symbol using the enclosing module name.
///
/// In Elixir, `defexception` is always called inside a module.  The exception type IS the
/// module itself, so we reuse the `qualified_prefix` tail as the symbol name — exactly the
/// same pattern as `defstruct`.
fn extract_exception(
    node: &Node,
    _src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let exception_name = qualified_prefix
        .rsplit('.')
        .next()
        .unwrap_or(qualified_prefix)
        .to_string();
    if exception_name.is_empty() {
        return;
    }
    let qualified_name = qualify(&exception_name, qualified_prefix);

    symbols.push(ExtractedSymbol {
        name: exception_name.clone(),
        qualified_name,
        kind: SymbolKind::Struct,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("defexception {exception_name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Protocol
// ---------------------------------------------------------------------------

fn extract_protocol(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let protocol_name = directive_target(node, src).unwrap_or_else(|| "Protocol".to_string());
    let qualified_name = qualify(&protocol_name, qualified_prefix);
    let new_prefix = qualified_name.clone();
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: protocol_name.clone(),
        qualified_name,
        kind: SymbolKind::Interface,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("defprotocol {protocol_name}")),
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
// Implementation (defimpl)
// ---------------------------------------------------------------------------

fn extract_implementation(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    // `defimpl ProtocolName, for: TargetType do ... end`
    // The first argument is the protocol name; `for:` option is the target type.
    let impl_name = directive_target(node, src).unwrap_or_else(|| "Impl".to_string());
    let qualified_name = qualify(&impl_name, qualified_prefix);
    let new_prefix = qualified_name.clone();
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: impl_name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("defimpl {impl_name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Emit TypeRef to the protocol being implemented
    refs.push(ExtractedRef {
        source_symbol_index: idx,
        target_name: impl_name,
        kind: EdgeKind::TypeRef,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
    });

    let do_block_idx = find_do_block_index(node);
    if let Some(i) = do_block_idx {
        if let Some(do_block) = node.child(i) {
            visit(do_block, src, symbols, refs, Some(idx), &new_prefix);
        }
    }
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
    let _ = directive;

    // Walk arguments to collect ALL alias/identifier children — handles both
    // single: `alias MyApp.User` and multi: `alias MyApp.{User, Post}`.
    let mut emitted = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "arguments" => {
                let mut ac = child.walk();
                for arg in child.children(&mut ac) {
                    match arg.kind() {
                        "alias" | "identifier" => {
                            let name = node_text(arg, src);
                            if !name.is_empty() {
                                let module = if name.contains('.') { Some(name.clone()) } else { None };
                                let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
                                refs.push(ExtractedRef {
                                    source_symbol_index: current_symbol_count,
                                    target_name: simple,
                                    kind: EdgeKind::Imports,
                                    line: arg.start_position().row as u32,
                                    module,
                                    chain: None,
                                });
                                emitted = true;
                            }
                        }
                        // `alias MyApp.{User, Post}` — list or tuple of aliases after a dot
                        "list" | "tuple" => {
                            let mut lc = arg.walk();
                            for item in arg.children(&mut lc) {
                                if item.kind() == "alias" || item.kind() == "identifier" {
                                    let name = node_text(item, src);
                                    if !name.is_empty() {
                                        let module = if name.contains('.') { Some(name.clone()) } else { None };
                                        let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
                                        refs.push(ExtractedRef {
                                            source_symbol_index: current_symbol_count,
                                            target_name: simple,
                                            kind: EdgeKind::Imports,
                                            line: item.start_position().row as u32,
                                            module,
                                            chain: None,
                                        });
                                        emitted = true;
                                    }
                                }
                            }
                        }
                        // `alias MyApp.{User, Post}` — tree-sitter-elixir represents this as:
                        //   arguments → dot { alias "MyApp" . tuple "{User, Post}" }
                        // The `dot` node has the module prefix and the right-side tuple of names.
                        "dot" => {
                            emitted |= extract_qualified_multi_alias(&arg, src, current_symbol_count, refs);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    // Fallback to directive_target if arguments walk didn't find anything.
    if !emitted {
        let target = directive_target(node, src).unwrap_or_default();
        if target.is_empty() {
            return;
        }
        let module = if target.contains('.') { Some(target.clone()) } else { None };
        let simple = target.rsplit('.').next().unwrap_or(&target).to_string();
        refs.push(ExtractedRef {
            source_symbol_index: current_symbol_count,
            target_name: simple,
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            module,
            chain: None,
        });
    }
}

/// Handle `alias MyApp.{User, Post}` — the `binary_operator` node for `.`
/// whose right side is a `tuple` or `list` containing the module names.
///
/// Returns true if at least one ref was emitted.
fn extract_qualified_multi_alias(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) -> bool {
    // The binary_operator for `MyApp.{User, Post}` has children:
    //   alias "MyApp"  .  tuple "{User, Post}"
    let children: Vec<tree_sitter::Node> = {
        let mut c = node.walk();
        node.children(&mut c).collect()
    };

    // Find the `.` operator.
    let dot_pos = children.iter().position(|c| node_text(*c, src) == ".");
    if dot_pos.is_none() {
        return false;
    }

    // The prefix is the left side (before `.`).
    let prefix = if let Some(left) = children.first() {
        node_text(*left, src)
    } else {
        return false;
    };

    // The right side (after `.`) should be a tuple or list: `{User, Post}`.
    let right = if let Some(dot_idx) = dot_pos {
        children.get(dot_idx + 1)
    } else {
        None
    };

    let right = match right {
        Some(r) => r,
        None => return false,
    };

    let mut emitted = false;
    if right.kind() == "tuple" || right.kind() == "list" || right.kind() == "keywords" {
        let mut rc = right.walk();
        for item in right.children(&mut rc) {
            if item.kind() == "alias" || item.kind() == "identifier" {
                let simple_name = node_text(item, src);
                if !simple_name.is_empty() {
                    let full_module = format!("{prefix}.{simple_name}");
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: simple_name,
                        kind: EdgeKind::Imports,
                        line: item.start_position().row as u32,
                        module: Some(full_module),
                        chain: None,
                    });
                    emitted = true;
                }
            }
        }
    } else if right.kind() == "alias" || right.kind() == "identifier" {
        // Fallback: `alias MyApp.User` as binary_operator form.
        let name = format!("{prefix}.{}", node_text(*right, src));
        let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
        refs.push(ExtractedRef {
            source_symbol_index: current_symbol_count,
            target_name: simple,
            kind: EdgeKind::Imports,
            line: right.start_position().row as u32,
            module: Some(name),
            chain: None,
        });
        emitted = true;
    }

    emitted
}

// ---------------------------------------------------------------------------
// Module attribute dispatch  (@moduledoc / @doc / @spec / other)
// ---------------------------------------------------------------------------

fn dispatch_attribute(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
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
            let sym_idx = symbols.len();
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
            // For @type and @spec, extract module references (alias nodes) as TypeRef edges.
            if attr_name == "type" || attr_name == "spec" || attr_name == "callback" {
                let ref_idx = parent_index.unwrap_or(sym_idx);
                extract_attribute_type_refs(node, src, ref_idx, refs);
            }
        }

        // `@behaviour GenServer` — emits a TypeRef edge (like implements)
        "behaviour" | "behavior" => {
            let target = extract_behaviour_target(node, src);
            if let Some(target_name) = target {
                // Use the parent symbol index if available; otherwise use current symbol count.
                let source_idx = parent_index.unwrap_or(symbols.len());
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
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
        match child.kind() {
            "call" => {
                if let Some(callee) = call_identifier(&child, src) {
                    if !matches!(callee.as_str(), "def" | "defp" | "defmacro" | "defmacrop" | "defmodule" | "defstruct" | "defexception" | "defprotocol" | "defimpl" | "defguard" | "defguardp" | "alias" | "import" | "use" | "require") {
                        let simple = callee.rsplit('.').next().unwrap_or(&callee).to_string();
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: simple,
                            kind: EdgeKind::Calls,
                            line: child.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                        // For dot calls like `Enum.map(...)`, also emit a TypeRef for
                        // the module part (the `alias` node before the dot).
                        extract_dot_call_module_ref(&child, src, source_symbol_index, refs);
                    }
                }
                extract_calls_recursive(&child, src, source_symbol_index, refs);
            }

            // Pipe operator: `value |> function_name(args)`
            "binary_operator" => {
                extract_pipe_calls(&child, src, source_symbol_index, refs);
                extract_calls_recursive(&child, src, source_symbol_index, refs);
            }

            // `alias` node (a capitalized module reference like `Enum`, `MyApp.User`).
            // Emit a TypeRef so that module references in expressions are tracked.
            "alias" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: simple,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: if name.contains('.') { Some(name) } else { None },
                        chain: None,
                    });
                }
            }

            // Anonymous functions: `fn arg -> body end` — recurse into stab_clause bodies.
            "anonymous_function" => {
                let mut fc = child.walk();
                for clause in child.children(&mut fc) {
                    if clause.kind() == "stab_clause" {
                        if let Some(body) = clause.child_by_field_name("body") {
                            extract_calls_recursive(&body, src, source_symbol_index, refs);
                        } else {
                            // fallback: last named child of stab_clause is the body
                            let clause_children: Vec<_> = {
                                let mut cc = clause.walk();
                                clause.named_children(&mut cc).collect()
                            };
                            if let Some(last) = clause_children.last() {
                                extract_calls_recursive(last, src, source_symbol_index, refs);
                            }
                        }
                    }
                }
            }

            // Keyword lists, maps, tuples, lists can contain calls — recurse.
            "keywords" | "keyword_list" | "map" | "tuple" | "list"
            | "arguments" | "body" | "block" | "do_block"
            | "access_call" | "unary_operator" => {
                extract_calls_recursive(&child, src, source_symbol_index, refs);
            }

            _ => {
                extract_calls_recursive(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// For a dot call like `Enum.map(...)`, emit a TypeRef to the receiver module.
///
/// The tree-sitter-elixir `call` node for `Enum.map(...)` has a `dot` child
/// whose first named child is the module (`alias` or `identifier`).
fn extract_dot_call_module_ref(
    call_node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = call_node.walk();
    for child in call_node.children(&mut cursor) {
        if child.kind() == "dot" {
            // `dot` → receiver (alias/identifier) . function_name
            let mut dc = child.walk();
            for dc_child in child.children(&mut dc) {
                match dc_child.kind() {
                    "alias" | "identifier" => {
                        let name = node_text(dc_child, src);
                        if !name.is_empty() {
                            let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: simple,
                                kind: EdgeKind::TypeRef,
                                line: dc_child.start_position().row as u32,
                                module: if name.contains('.') { Some(name) } else { None },
                                chain: None,
                            });
                        }
                        return; // only the receiver, not the function name
                    }
                    _ => {}
                }
            }
            return;
        }
    }
}

/// Emit a Calls edge for the right-hand side of a `|>` pipe expression.
fn extract_pipe_calls(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Collect all children to find the operator and operands.
    let children: Vec<tree_sitter::Node> = {
        let mut cursor = node.walk();
        node.children(&mut cursor).collect()
    };

    // Find the `|>` operator position.
    let pipe_pos = match children.iter().position(|c| node_text(*c, src) == "|>") {
        Some(p) => p,
        None => return, // not a pipe expression
    };

    // The right operand is the child after `|>`.
    let right = match children.get(pipe_pos + 1) {
        Some(r) => r,
        // Also try field name as a fallback.
        None => match node.child_by_field_name("right") {
            Some(r) => {
                let name = extract_pipe_callee_name(&r, src);
                if let Some(n) = name {
                    if !n.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: n,
                            kind: EdgeKind::Calls,
                            line: r.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                        // Also emit TypeRef for module part of dot calls on the right side.
                        extract_dot_call_module_ref(&r, src, source_symbol_index, refs);
                    }
                }
                return;
            }
            None => return,
        },
    };

    let name = extract_pipe_callee_name(right, src);
    if let Some(n) = name {
        if !n.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: n,
                kind: EdgeKind::Calls,
                line: right.start_position().row as u32,
                module: None,
                chain: None,
            });
            // Also emit TypeRef for module part of dot calls (`Enum.map`, etc.).
            extract_dot_call_module_ref(right, src, source_symbol_index, refs);
        }
    }
}

/// Extract the function name from the right side of a `|>` pipe.
///
/// Handles:
///   `validate(record)`       → "validate"   (identifier call)
///   `Enum.map(fn ...)`       → "map"         (dot-access call)
///   `String.length`          → "length"      (dot access, no parens)
///   `&String.upcase/1`       → "upcase"      (capture expression)
///   `&validate/1`            → "validate"    (bare capture)
fn extract_pipe_callee_name(node: &Node, src: &str) -> Option<String> {
    match node.kind() {
        "call" => {
            // Check if this is a dot-access call: `Enum.map(...)`
            // The call's first child is a `dot` node for qualified calls.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "dot" => {
                        // `dot` → alias/identifier . identifier
                        // The last identifier is the function name.
                        let mut dc = child.walk();
                        let mut last_ident: Option<String> = None;
                        for dc_child in child.children(&mut dc) {
                            if dc_child.kind() == "identifier" {
                                last_ident = Some(node_text(dc_child, src));
                            }
                        }
                        return last_ident;
                    }
                    "identifier" => {
                        // Bare call: `validate(...)`
                        return Some(node_text(child, src));
                    }
                    "alias" => {
                        // Module reference — use it as-is (shouldn't be a bare pipe target)
                        return Some(node_text(child, src));
                    }
                    _ => {}
                }
            }
            // Fallback to call_identifier
            call_identifier(node, src)
        }
        "identifier" => Some(node_text(*node, src)),
        "alias" => Some(node_text(*node, src)),
        // Capture expressions: `&String.upcase/1`, `&validate/1`
        // tree-sitter: unary_operator("&", binary_operator(dot_or_ident, "/", integer))
        "unary_operator" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "binary_operator" {
                    // The left side of the `/` is the function reference.
                    let children: Vec<_> = {
                        let mut c = child.walk();
                        child.children(&mut c).collect()
                    };
                    // Find `/` operator, take left side.
                    if let Some(slash_pos) = children.iter().position(|c| node_text(*c, src) == "/") {
                        if let Some(left) = children.get(slash_pos.saturating_sub(1)) {
                            // `left` may be a `call` (dot call) or `identifier` or `dot`.
                            return extract_pipe_callee_name(left, src);
                        }
                    }
                    // No slash — treat the whole expression as the name source.
                    return extract_pipe_callee_name(&child, src);
                }
                // `&identifier` without arity
                if child.kind() == "identifier" {
                    return Some(node_text(child, src));
                }
                if child.kind() == "call" || child.kind() == "dot" {
                    return extract_pipe_callee_name(&child, src);
                }
            }
            None
        }
        // `dot` node directly (qualified access without parens): `String.upcase`
        "dot" => {
            let mut cursor = node.walk();
            let mut last_ident: Option<String> = None;
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    last_ident = Some(node_text(child, src));
                }
            }
            last_ident
        }
        // Another binary_operator on the right side of a pipe: chained pipe expressions
        // that tree-sitter may represent as nested binary_operators.
        // Extract from the right operand of the nested pipe.
        "binary_operator" => {
            // If this binary_operator is itself a `|>`, extract the rightmost callee.
            let children: Vec<tree_sitter::Node> = {
                let mut c = node.walk();
                node.children(&mut c).collect()
            };
            if let Some(pipe_pos) = children.iter().position(|c| node_text(*c, src) == "|>") {
                if let Some(right) = children.get(pipe_pos + 1) {
                    return extract_pipe_callee_name(right, src);
                }
            }
            None
        }
        // Parenthesized expression: `(Module.fun)` — unwrap and recurse.
        "parenthesized_expression" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    let result = extract_pipe_callee_name(&child, src);
                    if result.is_some() {
                        return result;
                    }
                }
            }
            None
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Attribute type reference extraction  (@type / @spec / @callback)
// ---------------------------------------------------------------------------

/// Walk an attribute node and emit TypeRef edges for every `alias` node found
/// (module references like `GenServer.on_start`, `MyApp.User`, etc.).
fn extract_attribute_type_refs(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "alias" {
            let name = node_text(child, src);
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }
        extract_attribute_type_refs(&child, src, source_symbol_index, refs);
    }
}

// ---------------------------------------------------------------------------
// Behaviour target extraction
// ---------------------------------------------------------------------------

/// Extract the module name from a `@behaviour GenServer` unary_operator node.
///
/// Actual tree-sitter structure (tree-sitter-elixir):
///   unary_operator
///     "@"            ← anonymous token
///     call
///       identifier   "behaviour"
///       arguments
///         alias      "GenServer"
///
/// The `@` operator's operand is a `call` node with callee "behaviour" and the
/// target module as its sole argument.
fn extract_behaviour_target(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            // Verify this is @behaviour (not some other @attribute call)
            let callee = call_identifier(&child, src)?;
            if callee != "behaviour" && callee != "behavior" {
                return None;
            }
            // Extract the argument — the module being implemented.
            // Look for arguments field first, then walk children for arguments node.
            let args_node = if let Some(a) = child.child_by_field_name("arguments") {
                Some(a)
            } else {
                let mut cc = child.walk();
                let found = child.children(&mut cc).find(|c| c.kind() == "arguments");
                found
            };
            if let Some(args) = args_node {
                let mut ac = args.walk();
                for arg in args.children(&mut ac) {
                    match arg.kind() {
                        "alias" | "identifier" => return Some(node_text(arg, src)),
                        _ => {}
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Post-traversal full-tree type reference scan
// ---------------------------------------------------------------------------

/// Walk the entire CST and emit TypeRef edges for every `alias` node (module
/// references like `Enum`, `MyApp.User`) found in any context. Also walk
/// `dot` nodes to pick up the receiver module in `Module.function` calls.
///
/// This supplements the existing walker which only visits `alias` nodes that
/// appear as direct children of the nodes it explicitly handles.
///
/// No primitives to skip in Elixir — all alias nodes are module names.
fn scan_all_type_refs(node: tree_sitter::Node<'_>, src: &str, refs: &mut Vec<ExtractedRef>) {
    scan_type_refs_inner(node, src, 0, refs);
}

fn scan_type_refs_inner(
    node: tree_sitter::Node<'_>,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "alias" => {
            let name = node_text(node, src);
            if !name.is_empty() {
                let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: simple,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: if name.contains('.') { Some(name) } else { None },
                    chain: None,
                });
            }
            // alias is a leaf — no children to recurse into.
        }
        "dot" => {
            // `dot` node represents `Module.function` — emit TypeRef for the receiver.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "alias" | "identifier" => {
                        let name = node_text(child, src);
                        if !name.is_empty() {
                            // Only emit TypeRef if it looks like a module (starts uppercase or contains dot).
                            let first_char = name.chars().next().unwrap_or('_');
                            if first_char.is_uppercase() || name.contains('.') {
                                let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
                                refs.push(ExtractedRef {
                                    source_symbol_index,
                                    target_name: simple,
                                    kind: EdgeKind::TypeRef,
                                    line: child.start_position().row as u32,
                                    module: if name.contains('.') { Some(name) } else { None },
                                    chain: None,
                                });
                            }
                        }
                        break; // only the receiver (first child), not the function name
                    }
                    _ => {}
                }
            }
            // Still recurse into dot children for nested dots/aliases.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                scan_type_refs_inner(child, src, source_symbol_index, refs);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                scan_type_refs_inner(child, src, source_symbol_index, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

