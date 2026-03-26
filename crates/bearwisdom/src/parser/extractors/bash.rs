// =============================================================================
// parser/extractors/bash.rs  —  Bash / shell script extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function (function definitions — both POSIX `name() {}` and
//   `function name {}` / `function name() {}` forms)
//   Variable (simple assignments at file scope: `NAME=value`)
//
// REFERENCES:
//   - `source file` / `. file` → Imports edges
//   - Command calls within function bodies → Calls edges (best-effort;
//     the first word of a simple_command that resolves to a known name)
//
// Approach:
//   Single-pass recursive CST walk.  Bash is dynamically structured; we
//   match on node kinds produced by tree-sitter-bash 0.25:
//     function_definition, simple_command, source_command,
//     variable_assignment, command_name
//
// Note on coverage:
//   Bash is inherently hard to statically analyse.  We extract the structural
//   skeleton (functions, sourcing) rather than attempting full data-flow.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_bash::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Bash grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit(tree.root_node(), source, &mut symbols, &mut refs, None);

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
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                extract_function(&child, src, symbols, refs, parent_index);
            }

            // `source file` or `. file` at the top level
            "command" | "simple_command" => {
                // Check if this is a source/dot command
                if is_source_command(&child, src) {
                    extract_source_import(&child, src, symbols.len(), refs);
                } else if let Some(pi) = parent_index {
                    extract_command_call(&child, src, pi, refs);
                }
            }

            // Variable assignment at file scope: `NAME=value` or `NAME+=value`
            "variable_assignment" => {
                if parent_index.is_none() {
                    extract_variable(&child, src, symbols, parent_index);
                }
            }

            "ERROR" | "MISSING" => {}

            _ => {
                visit(child, src, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Function definition
// ---------------------------------------------------------------------------

fn extract_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // tree-sitter-bash: function_definition has a `name` field (word node)
    // and a `body` field (compound_statement / subshell).
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(n, src),
        None => match first_child_text_of_kind(node, src, "word") {
            Some(t) => t,
            None => return,
        },
    };

    let visibility = if name.starts_with('_') {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Function,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{name}()")),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    // Extract calls inside the function body
    if let Some(body) = node.child_by_field_name("body") {
        extract_body_refs(&body, src, idx, symbols, refs);
    }
}

/// Recursively visit a function body extracting source imports and calls.
fn extract_body_refs(
    node: &Node,
    src: &str,
    func_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "command" | "simple_command" => {
                if is_source_command(&child, src) {
                    extract_source_import(&child, src, func_idx, refs);
                } else {
                    extract_command_call(&child, src, func_idx, refs);
                }
            }
            "function_definition" => {
                // Nested function definition — extract as its own symbol
                extract_function(&child, src, symbols, refs, Some(func_idx));
            }
            "variable_assignment" => {
                // Local variables — skip (not exported as symbols)
            }
            _ => {
                extract_body_refs(&child, src, func_idx, symbols, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Variable assignment (file-scope)
// ---------------------------------------------------------------------------

fn extract_variable(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // variable_assignment: `NAME=value` — name field is the variable name
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(n, src),
        None => match first_child_text_of_kind(node, src, "variable_name") {
            Some(t) => t,
            None => return,
        },
    };

    if name.is_empty() {
        return;
    }

    let visibility = if name.starts_with('_') {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Variable,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(node_text(*node, src).lines().next().unwrap_or("").trim().to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Source / dot imports
// ---------------------------------------------------------------------------

/// True when `node` is a `source file` or `. file` command.
fn is_source_command(node: &Node, src: &str) -> bool {
    // The command name is the first word child.
    let cmd = first_word(node, src);
    cmd == "source" || cmd == "."
}

fn extract_source_import(
    node: &Node,
    src: &str,
    source_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Arguments after `source` / `.` are the file path(s).
    // They appear as `word` nodes after the command_name.
    let mut cursor = node.walk();
    let mut past_cmd = false;
    for child in node.children(&mut cursor) {
        if !past_cmd {
            if child.kind() == "command_name" || child.kind() == "word" {
                past_cmd = true;
                continue; // skip the `source` / `.` token itself
            }
        } else {
            if child.kind() == "word" || child.kind() == "string" {
                let raw = node_text(child, src)
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                if raw.is_empty() {
                    continue;
                }
                let target = raw
                    .rsplit('/')
                    .next()
                    .unwrap_or(&raw)
                    .trim_end_matches(".sh")
                    .to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: source_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(raw),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Command calls
// ---------------------------------------------------------------------------

/// Record a command invocation as a Calls edge.
/// We only record calls whose name looks like a shell function (identifier-ish),
/// skipping common builtins and control-flow keywords.
fn extract_command_call(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let cmd = first_word(node, src);
    if cmd.is_empty() {
        return;
    }
    // Skip builtins and syntax that isn't a function call
    if is_builtin(&cmd) {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: cmd,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
    });
}

/// Return the first word/command_name text of a command node.
fn first_word(node: &Node, src: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "command_name" => {
                // command_name wraps a word
                let mut cc = child.walk();
                for word in child.children(&mut cc) {
                    if word.kind() == "word" {
                        return node_text(word, src);
                    }
                }
                return node_text(child, src);
            }
            "word" => return node_text(child, src),
            _ => {}
        }
    }
    String::new()
}

/// Common shell builtins that aren't user-defined function calls.
fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "echo" | "printf" | "cd" | "pwd" | "ls" | "mkdir" | "rm" | "mv" | "cp"
            | "cat" | "grep" | "sed" | "awk" | "find" | "test" | "read" | "export"
            | "eval" | "exec" | "exit" | "return" | "break" | "continue"
            | "local" | "declare" | "typeset" | "readonly" | "set" | "unset"
            | "shift" | "trap" | "wait" | "kill" | "true" | "false" | ":" | "["
            | "[[" | "]]" | "]" | "if" | "then" | "else" | "fi" | "for" | "while"
            | "do" | "done" | "case" | "esac" | "in" | "function" | "source" | "."
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

/// Return the text of the first child whose kind matches `kind`.
/// Uses indexed child access to avoid cursor borrow lifetime issues.
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "bash_tests.rs"]
mod tests;
