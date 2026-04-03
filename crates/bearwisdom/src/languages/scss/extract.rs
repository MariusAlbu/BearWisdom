// =============================================================================
// languages/scss/extract.rs  —  SCSS / Sass extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — @mixin definitions, @function definitions, @keyframes
//   Variable  — $variable declarations at stylesheet scope
//   Class     — .class selectors in rule_set, %placeholder selectors
//
// REFERENCES:
//   Calls     — @include mixin-name, SCSS function call_expression
//   Inherits  — @extend .selector / @extend %placeholder
//   Imports   — @import, @use, @forward
//
// Grammar: tree-sitter-css 0.25 (handles SCSS subset).
// Node kinds used: stylesheet, rule_set, mixin_statement, function_statement,
//   keyframes_statement, include_statement, extend_statement,
//   import_statement, use_statement, forward_statement,
//   class_selector, id_selector, placeholder, declaration, variable_name,
//   call_expression, identifier, keyframes_name.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str, _file_path: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_css::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load CSS/SCSS grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // Walk top-level stylesheet children.
    let root = tree.root_node();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        visit_top_level(&child, source, &mut symbols, &mut refs);
    }

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Top-level traversal
// ---------------------------------------------------------------------------

fn visit_top_level(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "mixin_statement" => extract_mixin(node, src, symbols, refs),
        "function_statement" => extract_function_def(node, src, symbols, refs),
        "keyframes_statement" => extract_keyframes(node, src, symbols),
        "rule_set" => extract_rule_set(node, src, symbols, refs),
        "include_statement" => {
            // Top-level @include — attach to a sentinel index 0 (file scope)
            extract_include(node, src, 0, refs);
        }
        "extend_statement" => {
            extract_extend(node, src, 0, refs);
        }
        "import_statement" | "use_statement" | "forward_statement" => {
            extract_import(node, src, 0, refs);
        }
        "declaration" => {
            // Could be a top-level $variable declaration
            extract_variable_decl(node, src, symbols, None);
        }
        _ => {
            // Recurse into unknown wrappers (e.g. at_rule, media, supports)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                visit_top_level(&child, src, symbols, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// @mixin
// ---------------------------------------------------------------------------

fn extract_mixin(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(n, src),
        None => return,
    };

    let params = build_params_signature(node, src);
    let signature = format!("@mixin {name}{params}");

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name,
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(signature),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    // Extract refs inside mixin body
    extract_body_refs(node, src, idx, refs);
}

// ---------------------------------------------------------------------------
// @function
// ---------------------------------------------------------------------------

fn extract_function_def(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(n, src),
        None => return,
    };

    let params = build_params_signature(node, src);
    let signature = format!("@function {name}{params}");

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name,
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(signature),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    extract_body_refs(node, src, idx, refs);
}

// ---------------------------------------------------------------------------
// @keyframes
// ---------------------------------------------------------------------------

fn extract_keyframes(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    // keyframes_statement has a keyframes_name field or identifier child
    let name = node
        .child_by_field_name("keyframes_name")
        .map(|n| node_text(n, src))
        .or_else(|| find_child_text_of_kinds(node, src, &["identifier", "value"]))
        .unwrap_or_default();

    if name.is_empty() {
        return;
    }

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("@keyframes {name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
}

// ---------------------------------------------------------------------------
// rule_set (class selectors, id selectors, placeholder selectors)
// ---------------------------------------------------------------------------

fn extract_rule_set(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // Walk the selectors block to find class_selector, id_selector, placeholder
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "selectors" | "selector_list" => {
                extract_selectors(&child, src, symbols);
            }
            "class_selector" => {
                push_selector_symbol(&child, src, symbols, SymbolKind::Class, ".");
            }
            "id_selector" => {
                push_selector_symbol(&child, src, symbols, SymbolKind::Variable, "#");
            }
            "placeholder" => {
                push_selector_symbol(&child, src, symbols, SymbolKind::Class, "%");
            }
            "block" => {
                // Extract includes/extends inside the rule block
                let block_idx = symbols.len().saturating_sub(1);
                extract_body_refs(&child, src, block_idx, refs);
            }
            _ => {}
        }
    }
}

fn extract_selectors(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_selector" => push_selector_symbol(&child, src, symbols, SymbolKind::Class, "."),
            "id_selector" => push_selector_symbol(&child, src, symbols, SymbolKind::Variable, "#"),
            "placeholder" => push_selector_symbol(&child, src, symbols, SymbolKind::Class, "%"),
            _ => {}
        }
    }
}

fn push_selector_symbol(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    kind: SymbolKind,
    prefix: &str,
) {
    // class_selector has a class_name field; id_selector has id_name; placeholder has name
    let name = node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("class_name"))
        .or_else(|| node.child_by_field_name("id_name"))
        .map(|n| node_text(n, src))
        .or_else(|| {
            // Fallback: second child (after `.` or `#`) is the name token
            node.child(1).map(|n| node_text(n, src))
        })
        .unwrap_or_default();

    if name.is_empty() || name == prefix {
        return;
    }

    let display_name = format!("{prefix}{name}");
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(display_name),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
}

// ---------------------------------------------------------------------------
// $variable declaration (top-level)
// ---------------------------------------------------------------------------

fn extract_variable_decl(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // In the CSS grammar a top-level SCSS variable looks like:
    //   declaration → variable_name: <value>
    // variable_name child starts with `$`
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_name" {
            let raw = node_text(child, src);
            let name = raw.trim_start_matches('$').to_string();
            if name.is_empty() {
                continue;
            }
            let first_line = node_text(*node, src)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: name,
                kind: SymbolKind::Variable,
                visibility: Some(Visibility::Public),
                start_line: node.start_position().row as u32,
                end_line: node.end_position().row as u32,
                start_col: node.start_position().column as u32,
                end_col: node.end_position().column as u32,
                signature: Some(first_line),
                doc_comment: None,
                scope_path: None,
                parent_index,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Body reference extraction (@include, @extend, imports, function calls)
// ---------------------------------------------------------------------------

fn extract_body_refs(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "include_statement" => extract_include(&child, src, source_symbol_index, refs),
            "extend_statement" => extract_extend(&child, src, source_symbol_index, refs),
            "import_statement" | "use_statement" | "forward_statement" => {
                extract_import(&child, src, source_symbol_index, refs);
            }
            "call_expression" => extract_call_expr(&child, src, source_symbol_index, refs),
            _ => extract_body_refs(&child, src, source_symbol_index, refs),
        }
    }
}

// ---------------------------------------------------------------------------
// @include
// ---------------------------------------------------------------------------

fn extract_include(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // include_statement: @include <identifier> [(<args>)]
    let target = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .or_else(|| find_child_text_of_kinds(node, src, &["identifier", "function_name"]))
        .unwrap_or_default();

    if target.is_empty() {
        return;
    }

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
// @extend
// ---------------------------------------------------------------------------

fn extract_extend(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // extend_statement: @extend .class / @extend %placeholder
    // The target class/placeholder is a child node
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_selector" | "placeholder" => {
                let target = child
                    .child_by_field_name("name")
                    .or_else(|| child.child_by_field_name("class_name"))
                    .map(|n| node_text(n, src))
                    .or_else(|| child.child(1).map(|n| node_text(n, src)))
                    .unwrap_or_default();

                if !target.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: target,
                        kind: EdgeKind::Inherits,
                        line: node.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// @import / @use / @forward
// ---------------------------------------------------------------------------

fn extract_import(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // All three forms have a string_value child with the module path.
    let module = find_child_text_of_kinds(node, src, &["string_value", "string"])
        .map(|s| s.trim_matches('"').trim_matches('\'').to_string())
        .unwrap_or_default();

    if module.is_empty() {
        return;
    }

    // Derive a short target name (last path segment, no extension)
    let target = module
        .rsplit('/')
        .next()
        .unwrap_or(&module)
        .trim_start_matches('_')
        .trim_end_matches(".scss")
        .trim_end_matches(".sass")
        .trim_end_matches(".css")
        .to_string();

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: target,
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module: Some(module),
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// SCSS function call_expression
// ---------------------------------------------------------------------------

fn extract_call_expr(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let func_name = node
        .child_by_field_name("function_name")
        .or_else(|| node.child_by_field_name("name"))
        .or_else(|| node.child(0))
        .map(|n| node_text(n, src))
        .unwrap_or_default();

    if func_name.is_empty() {
        return;
    }

    // Strip namespace prefix (e.g. "math.ceil" → "ceil")
    let target = func_name
        .rsplit('.')
        .next()
        .unwrap_or(&func_name)
        .to_string();

    // Skip CSS built-ins that aren't user-defined SCSS functions
    if is_css_builtin(&target) {
        return;
    }

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

fn build_params_signature(node: &Node, src: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "parameters" {
            return format!("({})", node_text(child, src).trim_matches(|c| c == '(' || c == ')'));
        }
    }
    String::from("()")
}

fn find_child_text_of_kinds(node: &Node, src: &str, kinds: &[&str]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if kinds.contains(&child.kind()) {
            return Some(node_text(child, src));
        }
    }
    None
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

/// CSS/SCSS built-in functions — skip as calls targets (not user-defined).
fn is_css_builtin(name: &str) -> bool {
    matches!(
        name,
        "rgb" | "rgba" | "hsl" | "hsla" | "linear-gradient" | "radial-gradient"
            | "url" | "var" | "calc" | "env" | "min" | "max" | "clamp"
            | "translate" | "translateX" | "translateY" | "translateZ"
            | "scale" | "scaleX" | "scaleY" | "rotate" | "skew"
            | "blur" | "brightness" | "contrast" | "drop-shadow"
            | "grayscale" | "hue-rotate" | "invert" | "opacity" | "saturate"
            | "sepia" | "perspective" | "matrix" | "matrix3d"
            | "format" | "local" | "attr" | "counter" | "counters"
            | "cubic-bezier" | "steps" | "rect" | "polygon"
            | "circle" | "ellipse" | "inset" | "path"
    )
}
