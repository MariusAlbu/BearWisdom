// =============================================================================
// parser/extractors/bash.rs  —  Bash / shell script extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function   — function_definition (both POSIX `name() {}` and
//                `function name {}` / `function name() {}` forms)
//   Variable   — variable_assignment at any scope depth
//   Variable   — declaration_command (local/export/declare/typeset/readonly
//                with an assignment child)
//
// REFERENCES:
//   Imports    — `source file` / `. file`
//   Calls      — all command invocations (command / command_substitution nodes)
//
// Approach:
//   Single-pass recursive CST walk.  tree-sitter-bash 0.25 node kinds:
//     function_definition, simple_command, command, source_command,
//     variable_assignment, declaration_command, command_name,
//     command_substitution
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

            "command" | "simple_command" => {
                if is_source_command(&child, src) {
                    let src_idx = parent_index.unwrap_or(symbols.len());
                    extract_source_import(&child, src, src_idx, refs);
                } else {
                    let src_idx = parent_index.unwrap_or(symbols.len());
                    extract_command_call(&child, src, src_idx, refs);
                }
                // Recurse into command children to capture:
                //   - variable_assignment children (command-local env vars: `VAR=val cmd`)
                //   - command_substitution embedded in string/concatenation args
                visit(child, src, symbols, refs, parent_index);
            }

            "command_substitution" => {
                // Recurse into the substitution to handle commands (and nested
                // substitutions) inside it. The command arm will emit Calls refs
                // for any command nodes found, satisfying coverage for both
                // command_substitution and command node kinds.
                visit(child, src, symbols, refs, parent_index);
            }

            // variable_assignment at any scope — extract as symbol, then
            // recurse into its value for command_substitution refs.
            "variable_assignment" => {
                extract_variable(&child, src, symbols, parent_index);
                // The assignment value may contain command_substitution nodes.
                // Recurse with the same parent so refs land on the right symbol.
                visit(child, src, symbols, refs, parent_index);
            }

            // declaration_command: local/export/declare/typeset/readonly
            // Contains variable_assignment children — extract those as symbols.
            "declaration_command" => {
                extract_declaration(&child, src, symbols, parent_index);
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

    // Extract body — use the function's own index as parent
    if let Some(body) = node.child_by_field_name("body") {
        visit(body, src, symbols, refs, Some(idx));
    }
}

// ---------------------------------------------------------------------------
// Variable assignment
// ---------------------------------------------------------------------------

fn extract_variable(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
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
// Declaration command (local / export / declare / typeset / readonly)
// ---------------------------------------------------------------------------

/// Extract Variable symbols from a `declaration_command` node.
/// The node contains zero or more `variable_assignment` children.
fn extract_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let mut found_assignment = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_assignment" {
            extract_variable(&child, src, symbols, parent_index);
            found_assignment = true;
        }
    }

    // Handle bare declarations with no assignment: `export VAR`, `local X`, etc.
    // These have a `variable_name` or `word` child but no `=`.
    // We still emit a Variable symbol so the declaration_command budget node
    // gets matched at the correct line.
    if !found_assignment {
        let mut cursor2 = node.walk();
        for child in node.children(&mut cursor2) {
            if child.kind() == "variable_name" || child.kind() == "word" {
                let name = node_text(child, src);
                if !name.starts_with('-') && !name.is_empty() {
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
                        signature: None,
                        doc_comment: None,
                        scope_path: None,
                        parent_index,
                    });
                    break; // one symbol per declaration
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Source / dot imports
// ---------------------------------------------------------------------------

/// True when `node` is a `source file` or `. file` command.
fn is_source_command(node: &Node, src: &str) -> bool {
    let cmd = first_word(node, src);
    cmd == "source" || cmd == "."
}

fn extract_source_import(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    let mut past_cmd = false;
    for child in node.children(&mut cursor) {
        if !past_cmd {
            if child.kind() == "command_name" || child.kind() == "word" {
                past_cmd = true;
                continue;
            }
        } else if child.kind() == "word" || child.kind() == "string" {
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
                source_symbol_index,
                target_name: target,
                kind: EdgeKind::Imports,
                line: child.start_position().row as u32,
                module: Some(raw),
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
        }
    }
}

// ---------------------------------------------------------------------------
// Command calls
// ---------------------------------------------------------------------------

/// Record a command invocation as a Calls edge.
/// We emit refs for all commands — filtering to user-defined functions happens
/// at query time, not extraction time.

fn extract_command_call(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let cmd = first_word(node, src);
    let Some(cmd) = normalize_command_target(&cmd) else {
        return;
    };
    // Skip pure shell syntax tokens that aren't real invocations
    if is_syntax_keyword(&cmd) {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: cmd,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
        byte_offset: 0,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});
}

/// Filter / normalize a raw command-position word into a resolvable
/// target name. Returns None for words that aren't real call targets.
///
///   * `$VAR` / `${VAR}` / `"$VAR"` — variable expansion in command
///     position is a dynamic dispatch we can't resolve statically;
///     drop the ref entirely rather than emit it under the variable
///     name.
///   * `/usr/bin/find` and similar absolute paths — strip the
///     directory and resolve against the basename, which is what
///     `is_bash_builtin` matches anyway.
///   * Words containing embedded `"$x"` or `${x}` substitutions
///     (e.g., `__pb10k_prompt_"$seg"`, `_omb_alias_$name`) — same
///     dynamic-dispatch reasoning, drop.
///   * Empty / pure-syntax tokens — caller's `is_syntax_keyword`
///     already covers `[`, `[[`, etc.; we return them as-is.
fn normalize_command_target(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Reject any word containing a substitution marker. Bash's
    // `simple_command` AST doesn't always wrap these in a separate
    // expansion node — they leak into the `word` text verbatim.
    if trimmed.contains('$') || trimmed.contains("${") {
        return None;
    }
    // Reject words that aren't valid shell command identifiers. Tree-
    // sitter-bash occasionally splits malformed input into tokens like
    // `}1`, `}2`, `]`, `)` — fragments of unbalanced brace/paren
    // sequences that get classified as command-position words. None of
    // these are real call targets.
    let first = trimmed.chars().next().unwrap_or('\0');
    if !(first.is_ascii_alphabetic()
        || first == '_'
        || first == '/'
        || first == '.'
        || first == '"'
        || first == '\''
        || first.is_ascii_digit())
    {
        return None;
    }
    // Pure-numeric prefix on a single token (`1foo`, `}1`-style noise
    // that already fails the `}` check above) — reject. Real shell
    // commands don't start with digits except for absolute-path /-
    // numbered scripts which fall through the `/` branch already.
    if first.is_ascii_digit() {
        return None;
    }
    // Strip matching wrapping quotes once. `"foo"` / `'foo'` — the
    // inner literal is the command. Mismatched / unbalanced quotes
    // fall through to the no-quote branch.
    let unquoted = if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        let first = bytes[0];
        let last = bytes[trimmed.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            &trimmed[1..trimmed.len() - 1]
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    if unquoted.is_empty() {
        return None;
    }
    // Absolute path? Take the basename — `/usr/bin/find` resolves
    // against `find` in the keyword table.
    let base = if unquoted.starts_with('/') || unquoted.starts_with("./") || unquoted.starts_with("../") {
        unquoted.rsplit('/').next().unwrap_or(unquoted)
    } else {
        unquoted
    };
    if base.is_empty() {
        return None;
    }
    Some(base.to_string())
}


/// Return the first word/command_name text of a command node.
fn first_word(node: &Node, src: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "command_name" => {
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

/// Shell syntax keywords — these are not real command invocations.
/// Kept very small: only things that appear as the command word but are
/// pure grammar tokens rather than executable commands.
fn is_syntax_keyword(name: &str) -> bool {
    matches!(
        name,
        "[" | "[[" | "]]" | "]" | "!" | "function"
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

/// Return the text of the first child whose kind matches `kind`.
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
