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
    attribute_name, call_identifier, call_qualified_name, directive_target, find_do_block_index,
    function_name_arity, is_private_def, module_name_from_call, node_text, qualify,
    scope_from_prefix,
};

use super::calls::{extract_calls_recursive, extract_dot_call_module_ref, extract_pipe_calls};
use super::directives::extract_directive;
use super::type_refs::{extract_attribute_type_refs, extract_behaviour_target, scan_all_type_refs};

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

    // Dedup identical refs — `scan_all_type_refs` overlaps with per-arm
    // walks in `extract_node`. Key includes `module` so refs with
    // different module hints survive (a bare `foo` and a `MyMod.foo`
    // both at line 7 are semantically distinct). Same shape as
    // Kotlin / Scala.
    {
        let mut seen: std::collections::HashSet<(
            usize,
            String,
            crate::types::EdgeKind,
            u32,
            Option<String>,
        )> = std::collections::HashSet::with_capacity(refs.len());
        refs.retain(|r| {
            seen.insert((
                r.source_symbol_index,
                r.target_name.clone(),
                r.kind,
                r.line,
                r.module.clone(),
            ))
        });
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
                    col: 0,
                    module: if name.contains('.') { Some(name) } else { None },
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
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
            // Use the qualified form so the module prefix flows through
            // (`DateTime.utc_now` vs `NaiveDateTime.utc_now` etc.). The
            // tail-only `callee` from `call_identifier` would have lost
            // the prefix already.
            let qualified = call_qualified_name(node, src).unwrap_or_else(|| callee.clone());
            let name = qualified.rsplit('.').next().unwrap_or(&qualified).to_string();
            let module = qualified.rfind('.').map(|i| qualified[..i].to_string());
            refs.push(ExtractedRef {
                source_symbol_index: sym_idx,
                target_name: name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                col: 0,
                module,
                chain: None,
                byte_offset: node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
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
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
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
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
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
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
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
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
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
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
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
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

    // Emit TypeRef to the protocol being implemented
    refs.push(ExtractedRef {
        source_symbol_index: idx,
        target_name: impl_name,
        kind: EdgeKind::TypeRef,
        line: node.start_position().row as u32,
        col: 0,
        module: None,
        chain: None,
        byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});

    let do_block_idx = find_do_block_index(node);
    if let Some(i) = do_block_idx {
        if let Some(do_block) = node.child(i) {
            visit(do_block, src, symbols, refs, Some(idx), &new_prefix);
        }
    }
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
                            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
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
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }

        _ => {}
    }
}
