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
    // Its first argument is the function/macro name.
    let name = match extract_def_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let sig = build_def_signature(node, src);
    let idx = symbols.len();
    symbols.push(make_symbol(name.clone(), name, kind, node, Some(sig), None));

    // Recurse into the function body for nested calls.
    visit_def_body(node, src, idx, refs);
}

/// Extract the function/macro name: the first argument of the opening command.
fn extract_def_name(node: &Node, src: &str) -> Option<String> {
    // Walk to find the opening command node (function_command / macro_command).
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_command" | "macro_command" => {
                return first_argument_text(&child, src);
            }
            _ => {}
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
            let cmd = command_identifier(&child, src);
            if let Some(name) = cmd {
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }
        visit_def_body(&child, src, source_idx, refs);
    }
}

// ---------------------------------------------------------------------------
// normal_command dispatch
// ---------------------------------------------------------------------------

fn extract_normal_command(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let cmd = match command_identifier(node, src) {
        Some(c) => c,
        None => return,
    };

    // Emit a Calls edge for every command call (all commands are calls in CMake).
    let file_scope_idx = if symbols.is_empty() { 0 } else { symbols.len() - 1 };
    refs.push(ExtractedRef {
        source_symbol_index: file_scope_idx,
        target_name: cmd.clone(),
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
    });

    let cmd_lower = cmd.to_lowercase();
    match cmd_lower.as_str() {
        "set" => extract_set_command(node, src, symbols),
        "option" => extract_option_command(node, src, symbols),
        "add_executable" | "add_library" | "add_custom_target" => {
            extract_target_command(node, src, symbols, refs)
        }
        "project" => extract_project_command(node, src, symbols),
        "include" => extract_include_command(node, src, refs),
        "find_package" => extract_find_package_command(node, src, refs),
        "add_subdirectory" => extract_add_subdirectory_command(node, src, refs),
        "target_link_libraries" => extract_target_link_libraries(node, src, symbols, refs),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// set(<name> ...) → Variable
// ---------------------------------------------------------------------------

fn extract_set_command(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match nth_argument(node, src, 0) {
        Some(n) => n,
        None => return,
    };
    let sig = format!("set({} ...)", name);
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        Some(sig),
        None,
    ));
}

// ---------------------------------------------------------------------------
// option(<name> "description" <default>) → Variable
// ---------------------------------------------------------------------------

fn extract_option_command(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match nth_argument(node, src, 0) {
        Some(n) => n,
        None => return,
    };
    let sig = format!("option({} ...)", name);
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        Some(sig),
        None,
    ));
}

// ---------------------------------------------------------------------------
// add_executable / add_library / add_custom_target → Function (build target)
// ---------------------------------------------------------------------------

fn extract_target_command(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match nth_argument(node, src, 0) {
        Some(n) => n,
        None => return,
    };
    let cmd = command_identifier(node, src).unwrap_or_default();
    let sig = format!("{}({} ...)", cmd, name);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Function,
        node,
        Some(sig),
        None,
    ));
    // The command itself is already emitted as a Calls edge above in dispatch;
    // suppress a duplicate by not re-emitting here. The idx is stored for
    // target_link_libraries to reference.
    let _ = (idx, refs);
}

// ---------------------------------------------------------------------------
// project(<name> ...) → Namespace
// ---------------------------------------------------------------------------

fn extract_project_command(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match nth_argument(node, src, 0) {
        Some(n) => n,
        None => return,
    };
    let sig = format!("project({})", name);
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Namespace,
        node,
        Some(sig),
        None,
    ));
}

// ---------------------------------------------------------------------------
// include(<path>) → Imports
// ---------------------------------------------------------------------------

fn extract_include_command(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    let path = match nth_argument(node, src, 0) {
        Some(p) => p,
        None => return,
    };
    refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: path.clone(),
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module: Some(path),
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// find_package(<pkg> ...) → Imports
// ---------------------------------------------------------------------------

fn extract_find_package_command(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    let pkg = match nth_argument(node, src, 0) {
        Some(p) => p,
        None => return,
    };
    refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: pkg.clone(),
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module: Some(pkg),
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// add_subdirectory(<dir>) → Imports + Calls
// ---------------------------------------------------------------------------

fn extract_add_subdirectory_command(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    let dir = match nth_argument(node, src, 0) {
        Some(d) => d,
        None => return,
    };
    refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: dir.clone(),
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module: Some(dir),
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// target_link_libraries(<target> <libs...>) → Calls from target to each lib
// ---------------------------------------------------------------------------

fn extract_target_link_libraries(
    node: &Node,
    src: &str,
    symbols: &[ExtractedSymbol],
    refs: &mut Vec<ExtractedRef>,
) {
    // First argument is the target name; resolve to symbol index if possible.
    let target_name = match nth_argument(node, src, 0) {
        Some(n) => n,
        None => return,
    };

    // Find the target's symbol index, or default to 0.
    let target_idx = symbols
        .iter()
        .position(|s| s.name == target_name)
        .unwrap_or(0);

    // Remaining arguments are libraries (skip keywords like PUBLIC, PRIVATE, INTERFACE).
    let keywords = ["PUBLIC", "PRIVATE", "INTERFACE", "LINK_PUBLIC", "LINK_PRIVATE"];
    let args = collect_arguments(node, src);
    for (i, arg) in args.iter().enumerate() {
        if i == 0 {
            continue; // Skip target name.
        }
        let upper = arg.to_uppercase();
        if keywords.iter().any(|&k| k == upper.as_str()) {
            continue;
        }
        refs.push(ExtractedRef {
            source_symbol_index: target_idx,
            target_name: arg.clone(),
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Argument extraction helpers
// ---------------------------------------------------------------------------

/// Get the identifier of the command in a `normal_command` node.
fn command_identifier(node: &Node, src: &str) -> Option<String> {
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
fn first_argument_text(node: &Node, src: &str) -> Option<String> {
    nth_argument_from_node(node, src, 0)
}

/// Get the Nth argument from a command node.
fn nth_argument(node: &Node, src: &str, n: usize) -> Option<String> {
    nth_argument_from_node(node, src, n)
}

fn nth_argument_from_node(node: &Node, src: &str, n: usize) -> Option<String> {
    // Arguments are in an `argument_list` child, or directly as `argument`/`word` children.
    let args = collect_arguments(node, src);
    args.into_iter().nth(n)
}

/// Collect all argument texts from a command node.
fn collect_arguments(node: &Node, src: &str) -> Vec<String> {
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
                let text = node_text(child, src).trim().to_string();
                if !text.is_empty() {
                    args.push(text);
                }
            }
            "quoted_argument" => {
                // Strip surrounding quotes.
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
            node_text(*node, src).trim().to_string()
        }
        "quoted_argument" => {
            let raw = node_text(*node, src);
            raw.trim_matches('"').to_string()
        }
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn make_symbol(
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
    }
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}
