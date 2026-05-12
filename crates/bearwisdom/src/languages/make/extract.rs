// =============================================================================
// languages/make/extract.rs  —  Makefile / Make build system extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — rule targets (concrete and pattern rules)
//   Variable  — variable_assignment, define_directive, shell_assignment
//
// REFERENCES:
//   Calls     — rule prerequisites (dependency edges from target to each prerequisite)
//   Calls     — $(call func, ...) and $(shell ...) invocations
//   Imports   — include_directive (include path/to/file.mk)
//
// Grammar: tree-sitter-make (not yet in Cargo.toml — ready for when added).
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from a Makefile.
///
/// Wired into `MakePlugin::extract()` via tree-sitter-make.
pub fn extract(source: &str, language: tree_sitter::Language) -> crate::types::ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Make grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return crate::types::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let root = tree.root_node();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        extract_toplevel(&child, source, &mut symbols, &mut refs);
    }

    crate::types::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Top-level node dispatch
// ---------------------------------------------------------------------------

fn extract_toplevel(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // Skip nodes produced by error recovery — they are not real Make
    // constructs and would produce spurious symbols and refs.
    if node.is_error() || node.is_missing() {
        return;
    }
    match node.kind() {
        "rule" => extract_rule(node, src, symbols, refs),
        "variable_assignment" => extract_variable_assignment(node, src, symbols),
        "define_directive" => extract_define_directive(node, src, symbols),
        "shell_assignment" => extract_shell_assignment(node, src, symbols),
        "include_directive" => extract_include_directive(node, src, refs),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// rule: target: [prerequisites]
//         recipe...
// ---------------------------------------------------------------------------

fn extract_rule(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // Nodes created during error recovery are not valid Make rules.
    // Suppress them entirely to avoid spurious symbols and refs.
    if node.has_error() {
        return;
    }

    // Extract the first target name from the targets field.
    let target_name = match find_rule_target(node, src) {
        Some(n) => n,
        None => return,
    };

    // Build a signature from the raw first line up to the colon.
    let sig = build_rule_signature(node, src);

    let idx = symbols.len();
    symbols.push(make_symbol(
        target_name.clone(),
        target_name.clone(),
        SymbolKind::Function,
        node,
        Some(sig),
        None,
    ));

    // Special targets (`.PHONY`, `.SUFFIXES`, `.DEFAULT`, etc.) use their
    // prerequisite list as declarations, not as build dependencies. Emitting
    // Calls edges for them produces unresolvable refs against the declared
    // target names.
    //
    // Pattern rules (target contains `%`) also should not emit Calls edges
    // for their prerequisites — the prerequisite stems (`%.c`, `%.o`) are
    // not addressable symbols.
    if is_special_make_target(&target_name) || target_name.contains('%') {
        return;
    }

    // Prerequisites → Calls edges from the target to each dependency.
    extract_prerequisites(node, src, idx, refs);
}

/// Returns true for GNU Make / POSIX special targets that use their
/// prerequisite list as declarations rather than build dependencies.
/// Any target starting with `.` followed by all-uppercase ASCII letters
/// is treated as special.
pub(crate) fn is_special_make_target(name: &str) -> bool {
    if !name.starts_with('.') {
        return false;
    }
    let rest = &name[1..];
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_uppercase() || c == '_')
}

/// Find the first target word in a `rule` node's `targets` child.
fn find_rule_target(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "targets" || child.kind() == "target" {
            // First word or variable_reference child is the primary target.
            let mut cc = child.walk();
            for tc in child.children(&mut cc) {
                match tc.kind() {
                    "word" | "variable_reference" => {
                        let t = node_text(tc, src);
                        if !t.is_empty() {
                            return Some(t);
                        }
                    }
                    _ => {}
                }
            }
            // Fallback: use the full targets text.
            let t = node_text(child, src).trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    // Fallback: take text up to the first ':' in the node.
    let text = node_text(*node, src);
    if let Some(colon_pos) = text.find(':') {
        let target = text[..colon_pos].trim().to_string();
        if !target.is_empty() {
            return Some(target);
        }
    }
    None
}

/// Build a one-line signature for a rule: `<targets>: [prerequisites]`
fn build_rule_signature(node: &Node, src: &str) -> String {
    let text = node_text(*node, src);
    // Take the first line as the signature.
    text.lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Emit `Calls` edges for each prerequisite in a rule's prerequisites field.
fn extract_prerequisites(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "prerequisites" || child.kind() == "prerequisite" {
            let mut pc = child.walk();
            for prereq in child.children(&mut pc) {
                match prereq.kind() {
                    "word" | "variable_reference" => {
                        let name = node_text(prereq, src);
                        if !name.is_empty()
                            && !name.starts_with('$')
                            && !is_unresolvable_prereq(&name)
                        {
                            refs.push(ExtractedRef {
                                source_symbol_index: source_idx,
                                target_name: name,
                                kind: EdgeKind::Calls,
                                line: prereq.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                                namespace_segments: Vec::new(),
                                call_args: Vec::new(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        // Look for function_call nodes in the rule body (recipes).
        if child.kind() == "recipe" {
            extract_function_calls_in_subtree(&child, src, source_idx, refs);
        }
    }
}

/// Returns true for prerequisite tokens that cannot map to an indexed Make
/// target symbol and would always produce unresolved edges:
///
/// - Pattern stems (`%.c`, `%.o`) — wildcard build patterns, not targets.
/// - Unexpanded variable references (`$(FOO)/bar`) — still contain `$(`.
/// - File-path prerequisites (`src/main.o`, `foo.c`) — file-to-file build
///   deps tracked by the build system, not cross-target dependencies.
/// - Makefile filenames used as sentinel dependencies (`makefile`, `GNUmakefile`).
pub(crate) fn is_unresolvable_prereq(name: &str) -> bool {
    // Pattern stem.
    if name.contains('%') {
        return true;
    }
    // Unexpanded variable expansion remnant.
    if name.contains("$(") {
        return true;
    }
    // Path separator → file path dependency, not a target name.
    if name.contains('/') || name.contains('\\') {
        return true;
    }
    // Makefile filenames used as sentinel dependencies (rebuild when the
    // build file itself changes). Case-insensitive to cover all common forms.
    let lower = name.to_ascii_lowercase();
    if matches!(lower.as_str(), "makefile" | "gnumakefile" | "makefile.in") {
        return true;
    }
    // File extension that indicates a source or object file rather than a
    // named target. Only matches names of the form `stem.ext` where `ext`
    // is a known build artifact or source extension.
    if let Some(dot) = name.rfind('.') {
        let ext = &name[dot + 1..];
        // Single-char extensions like `.c`, `.h`, `.o`, `.a` plus common multi-char ones.
        matches!(
            ext,
            "c" | "h" | "cc" | "cpp" | "cxx" | "C" | "hh" | "hpp" | "hxx"
                | "o" | "obj" | "a" | "so" | "dylib" | "lib" | "dll"
                | "d" | "s" | "S" | "asm"
                | "f" | "f90" | "f95" | "for"
                | "go" | "rs" | "py" | "rb" | "js" | "ts" | "lua"
                | "java" | "class" | "jar"
                | "cs" | "vb"
                | "xml" | "xslt" | "xsl" | "dtd" | "xsls"
                | "html" | "htm" | "css" | "json" | "yaml" | "yml" | "toml"
                | "pod" | "pm" | "man" | "txt" | "md"
                | "mk" | "mak"
        )
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Variable assignments
// ---------------------------------------------------------------------------

fn extract_variable_assignment(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match find_field_text(node, src, "name") {
        Some(n) => n,
        None => first_word_in_node(node, src).unwrap_or_default(),
    };
    if name.is_empty() {
        return;
    }
    let sig = build_assignment_signature(node, src);
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        Some(sig),
        None,
    ));
}

fn extract_define_directive(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match find_field_text(node, src, "name") {
        Some(n) => n,
        None => {
            // `define VAR_NAME\n...\nendef` — take the word after `define`
            let text = node_text(*node, src);
            let after = text.trim_start_matches("define").trim();
            after.split_whitespace().next().unwrap_or("").to_string()
        }
    };
    if name.is_empty() {
        return;
    }
    let sig = format!("define {}", name);
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        Some(sig),
        None,
    ));
}

fn extract_shell_assignment(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match find_field_text(node, src, "name") {
        Some(n) => n,
        None => first_word_in_node(node, src).unwrap_or_default(),
    };
    if name.is_empty() {
        return;
    }
    let sig = build_assignment_signature(node, src);
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        Some(sig),
        None,
    ));
}

fn build_assignment_signature(node: &Node, src: &str) -> String {
    node_text(*node, src)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------
// include_directive → Imports
// ---------------------------------------------------------------------------

fn extract_include_directive(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    // The include_directive has a `filenames` field or word children.
    let paths = collect_include_paths(node, src);
    for path in paths {
        if !path.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: 0,
                target_name: path.clone(),
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module: Some(path),
                chain: None,
                byte_offset: 0,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }
    }
}

fn collect_include_paths(node: &Node, src: &str) -> Vec<String> {
    // Try the `filenames` named field first.
    if let Some(filenames) = node.child_by_field_name("filenames") {
        let text = node_text(filenames, src);
        return text
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
    }
    // Fallback: collect all word children after the `include` keyword.
    let mut paths = Vec::new();
    let text = node_text(*node, src);
    let after = text
        .trim_start_matches("-include")
        .trim_start_matches("sinclude")
        .trim_start_matches("include")
        .trim();
    for word in after.split_whitespace() {
        paths.push(word.to_string());
    }
    paths
}

// ---------------------------------------------------------------------------
// function_call / $(call ...) extraction
// ---------------------------------------------------------------------------

fn extract_function_calls_in_subtree(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "function_call" | "shell_function" => {
            if let Some(func_name) = find_field_text(node, src, "function") {
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: func_name,
                    kind: EdgeKind::Calls,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_function_calls_in_subtree(&child, src, source_idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the text of a named field child.
fn find_field_text(node: &Node, src: &str, field: &str) -> Option<String> {
    node.child_by_field_name(field).map(|n| node_text(n, src))
}

/// Get the text of the first `word` child in a node.
fn first_word_in_node(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "word" {
            let t = node_text(child, src);
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

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
