// =============================================================================
// languages/scss/extract.rs  —  SCSS / Sass extractor
//
// Grammar: tree-sitter-scss-local (dedicated SCSS grammar, MSVC-compatible
//   via pre-expanded parser_expanded.c). The SCSS grammar has proper nodes
//   for every SCSS construct; no CSS grammar fallback needed.
//
// SYMBOLS:
//   Function  — mixin_statement, function_statement, keyframes_statement
//   Class     — rule_set (selectors)
//   Variable  — declaration with $variable LHS
//
// REFERENCES:
//   Calls     — include_statement, call_expression
//   Inherits  — extend_statement
//   Imports   — import_statement, forward_statement
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str, _file_path: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_scss_local::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load SCSS grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let root = tree.root_node();
    visit_node(&root, source, &mut symbols, &mut refs, None);

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Tree walker — dispatches on SCSS grammar node kinds
// ---------------------------------------------------------------------------

fn visit_node(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    match node.kind() {
        "mixin_statement" => {
            handle_mixin(node, src, symbols, refs);
        }
        "function_statement" => {
            handle_function(node, src, symbols, refs);
        }
        "include_statement" => {
            let sym_idx = symbols.len();
            handle_include(node, src, refs, symbols, sym_idx);
        }
        "extend_statement" => {
            handle_extend(node, src, refs, symbols.len());
        }
        "import_statement" => {
            let sym_idx = symbols.len();
            handle_import(node, src, refs, symbols, sym_idx);
        }
        "forward_statement" => {
            let sym_idx = symbols.len();
            handle_forward(node, src, refs, symbols, sym_idx);
        }
        "keyframes_statement" => {
            handle_keyframes(node, src, symbols, refs);
        }
        "rule_set" => {
            handle_rule_set(node, src, symbols, refs, parent_idx);
        }
        "declaration" => {
            handle_declaration(node, src, symbols, refs, parent_idx);
        }
        "call_expression" => {
            handle_call_expr(node, src, refs, symbols.len());
        }
        _ => {
            // Recurse into all other nodes (stylesheet, block, media_statement, etc.)
            visit_children(node, src, symbols, refs, parent_idx);
        }
    }
}

fn visit_children(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            visit_node(&child, src, symbols, refs, parent_idx);
        }
    }
}

// ---------------------------------------------------------------------------
// @mixin name { ... }  =>  Function symbol + recurse body
// ---------------------------------------------------------------------------

fn handle_mixin(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .unwrap_or_default();
    if name.is_empty() {
        return;
    }

    let idx = symbols.len();
    symbols.push(make_sym(
        name.clone(),
        SymbolKind::Function,
        node,
        None,
        Some(format!("@mixin {name}")),
    ));

    // Recurse into all children (parameters with defaults, block body)
    visit_children(node, src, symbols, refs, Some(idx));
}

// ---------------------------------------------------------------------------
// @function name($args) { ... }  =>  Function symbol + recurse body
// ---------------------------------------------------------------------------

fn handle_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .unwrap_or_default();
    if name.is_empty() {
        return;
    }

    let idx = symbols.len();
    symbols.push(make_sym(
        name.clone(),
        SymbolKind::Function,
        node,
        None,
        Some(format!("@function {name}")),
    ));

    // Recurse into all children (parameters with defaults, block body)
    visit_children(node, src, symbols, refs, Some(idx));
}

// ---------------------------------------------------------------------------
// @include mixin-name(args)  =>  Calls ref
// ---------------------------------------------------------------------------

fn handle_include(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    symbols: &mut Vec<ExtractedSymbol>,
    source_symbol_index: usize,
) {
    let target = find_first_identifier(node, src);
    if !target.is_empty() {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
    // Recurse into arguments to find nested call_expressions
    visit_children(node, src, symbols, refs, Some(source_symbol_index));
}

// ---------------------------------------------------------------------------
// @extend .selector / %placeholder  =>  Inherits ref
// ---------------------------------------------------------------------------

fn handle_extend(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    source_symbol_index: usize,
) {
    let target = find_selector_target(node, src);
    if target.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: target,
        kind: EdgeKind::Inherits,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// @import 'path'  =>  Imports ref
// ---------------------------------------------------------------------------

fn handle_import(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    symbols: &mut Vec<ExtractedSymbol>,
    source_symbol_index: usize,
) {
    let module = find_string_value(node, src);
    if !module.is_empty() {
        let target = path_to_target(&module);
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            module: Some(module),
            chain: None,
        });
    }
    visit_children(node, src, symbols, refs, Some(source_symbol_index));
}

// ---------------------------------------------------------------------------
// @forward 'path'  =>  Imports ref
// ---------------------------------------------------------------------------

fn handle_forward(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    symbols: &mut Vec<ExtractedSymbol>,
    source_symbol_index: usize,
) {
    let module = find_string_value(node, src);
    if !module.is_empty() {
        let target = path_to_target(&module);
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            module: Some(module),
            chain: None,
        });
    }
    visit_children(node, src, symbols, refs, Some(source_symbol_index));
}

// ---------------------------------------------------------------------------
// @keyframes name { ... }  =>  Function symbol
// ---------------------------------------------------------------------------

fn handle_keyframes(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = find_child_of_kind(node, "keyframes_name")
        .map(|n| node_text(n, src))
        .or_else(|| find_child_of_kind(node, "identifier").map(|n| node_text(n, src)))
        .unwrap_or_default();

    let idx = symbols.len();
    if !name.is_empty() {
        symbols.push(make_sym(
            name.clone(),
            SymbolKind::Function,
            node,
            None,
            Some(format!("@keyframes {name}")),
        ));
    }

    // Recurse into keyframe_block_list for any nested call_expressions
    visit_children(node, src, symbols, refs, Some(idx));
}

// ---------------------------------------------------------------------------
// rule_set { selectors { block } }  =>  Class symbol per rule set
// ---------------------------------------------------------------------------

fn handle_rule_set(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    let name = find_child_of_kind(node, "selectors")
        .and_then(|sel| extract_first_selector_name(&sel, src))
        .or_else(|| {
            let row = node.start_position().row;
            src.lines().nth(row).and_then(|line| {
                let trimmed = line.trim();
                let name = trimmed
                    .split(|c: char| c == '{' || c == ',' || c == ' ')
                    .next()
                    .unwrap_or("")
                    .trim_start_matches('.')
                    .trim_start_matches('#')
                    .trim_start_matches('%')
                    .trim_start_matches('&');
                if name.is_empty() {
                    None
                } else {
                    Some(name.to_string())
                }
            })
        });

    let idx = symbols.len();
    if let Some(name) = name {
        let clean = name
            .trim_start_matches('.')
            .trim_start_matches('#')
            .trim_start_matches('%')
            .trim_start_matches('&')
            .to_string();
        let display = if clean.is_empty() { name } else { clean };
        symbols.push(make_sym(display, SymbolKind::Class, node, parent_idx, None));
    } else {
        visit_children(node, src, symbols, refs, parent_idx);
        return;
    }

    // Recurse into all children (selectors may contain pseudo-class call_expressions,
    // block contains nested rules and declarations)
    visit_children(node, src, symbols, refs, Some(idx));
}

// ---------------------------------------------------------------------------
// declaration: $variable: value  =>  Variable symbol
// ---------------------------------------------------------------------------

fn handle_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    // First child of declaration is the property_name (or variable).
    // If it starts with $ it's an SCSS variable declaration.
    if let Some(prop) = node.child(0) {
        let raw = node_text(prop, src);
        if raw.starts_with('$') {
            let name = raw.trim_start_matches('$').to_string();
            if !name.is_empty() {
                let first_line = src
                    .lines()
                    .nth(node.start_position().row)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                symbols.push(make_sym(
                    name,
                    SymbolKind::Variable,
                    node,
                    parent_idx,
                    Some(first_line),
                ));
            }
            // Still recurse to find call_expressions in the value
        }
    }
    // Recurse into all children to find nested call_expressions and refs
    visit_children(node, src, symbols, refs, parent_idx);
}

// ---------------------------------------------------------------------------
// call_expression  =>  Calls ref
// ---------------------------------------------------------------------------

fn handle_call_expr(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    source_symbol_index: usize,
) {
    // Extract the function name from the call_expression node.
    // The function_name child is a leaf with the function identifier text.
    let func_name = find_child_of_kind(node, "function_name")
        .map(|n| node_text(n, src))
        .or_else(|| node.child(0).map(|n| {
            let t = node_text(n, src);
            // Extract identifier from interpolation or other non-leaf
            t.trim_matches('#').trim_matches('{').trim_matches('}').to_string()
        }))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "<call>".to_string());

    let target = func_name
        .rsplit('.')
        .next()
        .unwrap_or(&func_name)
        .trim()
        .to_string();

    let target = if target.is_empty() {
        "<call>".to_string()
    } else {
        target
    };

    // Emit Calls refs for all call expressions including CSS builtins.
    // The graph engine resolves which targets are user-defined; builtins
    // will simply have no matching symbol and be treated as external calls.
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: target,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_sym(
    name: String,
    kind: SymbolKind,
    node: &Node,
    parent_index: Option<usize>,
    signature: Option<String>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.clone(),
        qualified_name: name,
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

fn find_child_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == kind {
                return Some(child);
            }
        }
    }
    None
}

fn find_first_identifier(node: &Node, src: &str) -> String {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "identifier" {
                return node_text(child, src);
            }
        }
    }
    String::new()
}

fn find_selector_target(node: &Node, src: &str) -> String {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "class_selector" => {
                    if let Some(cn) = child
                        .child_by_field_name("class_name")
                        .or_else(|| child.child(1))
                    {
                        return node_text(cn, src);
                    }
                }
                "placeholder" => {
                    if let Some(cn) = child.child(1) {
                        return node_text(cn, src);
                    }
                }
                "identifier" => {
                    return node_text(child, src);
                }
                _ => {}
            }
        }
    }
    String::new()
}

fn find_string_value(node: &Node, src: &str) -> String {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "string_value" {
                let raw = node_text(child, src);
                return raw.trim_matches('"').trim_matches('\'').to_string();
            }
        }
    }
    String::new()
}

fn path_to_target(module: &str) -> String {
    module
        .rsplit('/')
        .next()
        .unwrap_or(module)
        .trim_start_matches('_')
        .trim_end_matches(".scss")
        .trim_end_matches(".sass")
        .trim_end_matches(".css")
        .to_string()
}

fn extract_first_selector_name(node: &Node, src: &str) -> Option<String> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "class_selector" => {
                    let name = child
                        .child_by_field_name("class_name")
                        .or_else(|| child.child(1))
                        .map(|n| node_text(n, src))?;
                    if !name.is_empty() {
                        return Some(format!(".{name}"));
                    }
                }
                "id_selector" => {
                    let name = child
                        .child_by_field_name("id_name")
                        .or_else(|| child.child(1))
                        .map(|n| node_text(n, src))?;
                    if !name.is_empty() {
                        return Some(format!("#{name}"));
                    }
                }
                "placeholder" => {
                    let name = child.child(1).map(|n| node_text(n, src))?;
                    if !name.is_empty() {
                        return Some(format!("%{name}"));
                    }
                }
                "tag_name" | "nesting_selector" | "universal_selector" => {
                    let t = node_text(child, src);
                    if !t.is_empty() {
                        return Some(t);
                    }
                }
                "pseudo_class_selector" | "pseudo_element_selector" => {
                    let t = node_text(child, src);
                    if !t.is_empty() && !t.contains('{') {
                        return Some(t);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

