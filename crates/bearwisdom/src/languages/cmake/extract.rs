// =============================================================================
// languages/cmake/extract.rs  —  CMake build system extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — function_def, macro_def
//   Function  — add_executable, add_library, add_custom_target (build targets)
//   Variable  — set(<name> ...) and option(<name> ...) at top level
//   Namespace — project(<name> ...)
//
// REFERENCES:
//   Calls     — every normal_command → command identifier
//   Imports   — include(<path>), find_package(<pkg>), add_subdirectory(<dir>)
//
// Grammar: tree-sitter-cmake (not yet in Cargo.toml — ready for when added).
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use super::arguments::{collect_arguments, command_identifier, first_argument_text, normalize_argument};
use super::commands::{collect_all_normal_commands, extract_normal_command};
use super::hooks::is_cmake_builtin;
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from a CMakeLists.txt / .cmake file.
///
/// Requires the tree-sitter-cmake grammar to be available as `language`.
/// Called by `CMakePlugin::extract()` once the grammar is wired in.
#[allow(dead_code)]
pub fn extract(source: &str, language: tree_sitter::Language) -> crate::types::ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load CMake grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return crate::types::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit_source_file(tree.root_node(), source, &mut symbols, &mut refs);

    // Second pass: collect all variable_ref nodes for ref coverage
    collect_variable_refs(tree.root_node(), source, &mut refs);

    // Third pass: collect all normal_command nodes not yet matched (inside function/macro bodies)
    let cmd_lines: std::collections::HashSet<u32> = symbols.iter().map(|s| s.start_line).collect();
    collect_all_normal_commands(tree.root_node(), source, &cmd_lines, &mut symbols, &mut refs);

    crate::types::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Top-level traversal
// ---------------------------------------------------------------------------

fn visit_source_file(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "source_file" | "body" => visit_source_file(child, src, symbols, refs),
            "function_def" => extract_function_def(&child, src, symbols, refs, SymbolKind::Function),
            "macro_def" => extract_function_def(&child, src, symbols, refs, SymbolKind::Function),
            "normal_command" => extract_normal_command(&child, src, symbols, refs),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// function_def / macro_def → Function
// ---------------------------------------------------------------------------

fn extract_function_def(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    kind: SymbolKind,
) {
    // The opening command is `function_command` or `macro_command`.
    // Its first argument is the function/macro name; subsequent args are
    // parameter names that become local variables in the function body.
    let opening = find_opening_command(node);
    let name = match opening.as_ref().and_then(|c| first_argument_text(c, src)) {
        Some(n) => n,
        None => return,
    };

    let sig = build_def_signature(node, src);
    let idx = symbols.len();
    symbols.push(make_symbol(name.clone(), name, kind, node, Some(sig), None));

    // Capture parameters (args 1..N of function_command/macro_command) as Variable
    // symbols. They are referenced inside the body via `${PARAM}`.
    if let Some(cmd) = opening {
        let args = collect_arguments(&cmd, src);
        for param in args.into_iter().skip(1) {
            if param.is_empty() || param.starts_with('$') {
                continue;
            }
            let param_sig = format!("{}() parameter ${}", "fn", param);
            symbols.push(make_symbol(
                param.clone(),
                param,
                SymbolKind::Variable,
                &cmd,
                Some(param_sig),
                Some(idx),
            ));
        }
    }

    // Recurse into the function body for nested calls.
    visit_def_body(node, src, idx, refs);
}

fn find_opening_command<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "function_command" | "macro_command") {
            return Some(child);
        }
    }
    None
}

fn build_def_signature(node: &Node, src: &str) -> String {
    // Use the first line (the opening command) as the signature.
    node_text(*node, src)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Recurse into the body of a function/macro def, emitting Calls refs.
fn visit_def_body(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "normal_command" {
            if let Some(name) = command_identifier(&child, src) {
                // Only emit Calls for user-defined (non-builtin) commands.
                if !is_cmake_builtin(&name) {
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index: source_idx,
                        target_name: name,
                        kind: EdgeKind::Calls,
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
        }
        visit_def_body(&child, src, source_idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub(super) fn make_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    node: &Node,
    signature: Option<String>,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index,
    byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
}
}

/// Walk the entire tree and emit a TypeRef for every `variable_ref` node.
/// Generator expressions (`$<...>`) are skipped.
/// This second pass ensures coverage correlation finds a ref for every
/// variable_ref occurrence (the ref_node_kind).
fn collect_variable_refs(
    node: Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() == "variable_ref" {
        let raw = node_text(node, src);
        // Skip generator expressions
        if raw.starts_with("$<") {
            return;
        }
        let name = extract_variable_ref_name(&node, src);
        let target = if name.is_empty() {
            normalize_argument(&raw)
        } else {
            name
        };
        // Names containing `}` are extraction artifacts of nested variable
        // references like `${${OUTER}_SUFFIX}` — the inner identifier plus the
        // unparsed tail leaks into the name.  They cannot be resolved and
        // add noise to unresolved-ref counts.
        if !target.is_empty() && !target.contains('}') {
            refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                source_symbol_index: 0,
                target_name: target,
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
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_variable_refs(child, src, refs);
    }
}

/// Extract the variable name from a `variable_ref` node (strips `${}` syntax).
fn extract_variable_ref_name(node: &Node, src: &str) -> String {
    // variable_ref grammar: `${` identifier `}` or `$ENV{` identifier `}` etc.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "variable" {
            let t = node_text(child, src).trim().to_string();
            if !t.is_empty() {
                return t;
            }
        }
    }
    // Fallback: strip ${ and }
    let raw = node_text(*node, src);
    raw.trim_start_matches("${")
        .trim_start_matches("$ENV{")
        .trim_start_matches("$CACHE{")
        .trim_end_matches('}')
        .trim()
        .to_string()
}

pub(super) fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}
