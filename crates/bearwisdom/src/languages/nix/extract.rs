// =============================================================================
// languages/nix/extract.rs  —  Nix expression language extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Variable  — binding where value is a non-function expression
//   Function  — binding where value is a function_expression (lambda)
//   Variable  — inherit / inherit_from items
//
// REFERENCES:
//   Imports   — apply_expression calling `import` → path argument
//   Imports   — apply_expression calling `callPackage` → first path argument
//   Imports   — with_expression → the environment name (brings scope into context)
//   Calls     — apply_expression → function name (variable_expression / select_expression)
//
// Grammar: tree-sitter-nix (not yet in Cargo.toml — ready for when added).
// Nix is purely functional; every construct is an expression. The primary
// declaration form is `binding` inside attrset/let expressions.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

use super::bindings::{
    binding_name, binding_value, extract_inherit, extract_inherit_from, is_function_expr,
};
use super::calls::{extract_apply, extract_with, resolve_call_name, visit_formal_defaults};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from a Nix expression file.
///
/// Requires the tree-sitter-nix grammar to be available as `language`.
/// Called by `NixPlugin::extract()` once the grammar is wired in.
#[allow(dead_code)]
pub fn extract(source: &str, language: tree_sitter::Language) -> crate::types::ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Nix grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return crate::types::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // The root of a Nix file is typically a single expression.
    // We walk the whole tree to capture top-level and let-bound symbols.
    visit_expr(
        tree.root_node(),
        source,
        &mut symbols,
        &mut refs,
        None,
        true,
    );

    crate::types::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Expression traversal
// ---------------------------------------------------------------------------

/// Visit a Nix expression, extracting symbols and refs.
/// `top_level` is true when visiting the outermost expression in the file,
/// which controls whether bindings in attribute sets are treated as public.
fn visit_expr(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    top_level: bool,
) {
    match node.kind() {
        "attrset_expression" | "rec_attrset_expression" => {
            extract_attrset(node, src, symbols, refs, parent_index, top_level);
        }
        "let_expression" | "let_attrset_expression" => {
            extract_let(node, src, symbols, refs, parent_index);
        }
        "with_expression" => {
            extract_with(node, src, symbols.len(), refs);
            // Continue into the body
            if let Some(body) = node.child_by_field_name("body") {
                visit_expr(body, src, symbols, refs, parent_index, false);
            }
        }
        "apply_expression" => {
            let source_idx = symbols.len().saturating_sub(1);
            extract_apply(node, src, source_idx, refs);
            // Recurse into sub-expressions (chained applies, argument expressions)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    visit_expr(child, src, symbols, refs, parent_index, false);
                }
            }
        }
        "select_expression" => {
            // Emit a Calls ref for every select_expression
            let source_idx = symbols.len().saturating_sub(1);
            let name = resolve_call_name(node, src).unwrap_or_else(|| node_text(node, src));
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: source_idx,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: node.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
            // Recurse into sub-expressions
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    visit_expr(child, src, symbols, refs, parent_index, false);
                }
            }
        }
        "function_expression" => {
            // A lambda — not a named declaration at this level.
            // Visit formal parameter default values, then the body.
            visit_formal_defaults(node, src, symbols, refs, parent_index);
            if let Some(body) = node.child_by_field_name("body") {
                visit_expr(body, src, symbols, refs, parent_index, false);
            }
        }
        _ => {
            // Descend into child expressions
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    visit_expr(child, src, symbols, refs, parent_index, false);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Attribute set  (top-level or rec attrset)
// ---------------------------------------------------------------------------

fn extract_attrset(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    is_public: bool,
) {
    let vis = if is_public {
        Visibility::Public
    } else {
        Visibility::Private
    };

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "binding_set" => {
                // tree-sitter-nix 0.3: bindings are wrapped in binding_set
                extract_binding_set(child, src, symbols, refs, parent_index, vis);
            }
            "binding" => {
                extract_binding(&child, src, symbols, refs, parent_index, vis);
            }
            "inherit" => {
                extract_inherit(&child, src, symbols, parent_index, vis);
            }
            "inherit_from" => {
                extract_inherit_from(&child, src, symbols, refs, parent_index, vis);
            }
            _ => {}
        }
    }
}

/// Extract bindings from a `binding_set` node.
fn extract_binding_set(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    vis: Visibility,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "binding" => {
                extract_binding(&child, src, symbols, refs, parent_index, vis);
            }
            "inherit" => {
                extract_inherit(&child, src, symbols, parent_index, vis);
            }
            "inherit_from" => {
                extract_inherit_from(&child, src, symbols, refs, parent_index, vis);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Let expression  (let ... in ...)
// ---------------------------------------------------------------------------

fn extract_let(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // Bindings in `let` are private (local scope)
    let vis = Visibility::Private;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "binding_set" => {
                let mut bc = child.walk();
                for binding in child.children(&mut bc) {
                    match binding.kind() {
                        "binding" => {
                            extract_binding(&binding, src, symbols, refs, parent_index, vis);
                        }
                        "inherit" => {
                            extract_inherit(&binding, src, symbols, parent_index, vis);
                        }
                        "inherit_from" => {
                            extract_inherit_from(&binding, src, symbols, refs, parent_index, vis);
                        }
                        _ => {}
                    }
                }
            }
            "binding" => {
                extract_binding(&child, src, symbols, refs, parent_index, vis);
            }
            _ => {
                // `in` body expression — visit for nested lets/attrsets
                if is_expr_node(&child) && child.kind() != "let_expression" {
                    visit_expr(child, src, symbols, refs, parent_index, false);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Binding  (name = expr;)
// ---------------------------------------------------------------------------

fn extract_binding(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    vis: Visibility,
) {
    // attrpath gives the binding name (may be dotted: a.b.c).
    // For bindings with interpolated attrpaths (e.g. `${name} = expr`), the
    // name may not be statically extractable. In that case, still process the
    // value expression for refs — skip only the symbol creation.
    let name_opt = binding_name(node, src);

    let value = binding_value(node);

    let idx = if let Some(name) = name_opt {
        let kind = match value {
            Some(v) if is_function_expr(v) => SymbolKind::Function,
            _ => SymbolKind::Variable,
        };
        let sig = if kind == SymbolKind::Function {
            format!("{} = ...: ...", name)
        } else {
            format!("{} = ...", name)
        };
        let i = symbols.len();
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name,
            kind,
            visibility: Some(vis),
            start_line: node.start_position().row as u32,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: node.end_position().column as u32,
            signature: Some(sig),
            doc_comment: None,
            scope_path: None,
            parent_index,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });
        i
    } else {
        // Name not statically extractable (interpolated attrpath).
        // Use the nearest parent symbol as the ref source.
        parent_index.unwrap_or(symbols.len().saturating_sub(1))
    };

    // Visit the value expression for nested declarations and refs
    if let Some(v) = value {
        extract_value_refs(v, src, idx, symbols, refs);
    }
}

/// Extract refs and nested symbols from a binding's value expression.
pub(super) fn extract_value_refs(
    node: Node,
    src: &str,
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "apply_expression" => {
            extract_apply(node, src, source_symbol_index, refs);
            // Recurse into all sub-expressions, including nested apply_expression
            // children. This handles curried application (`f a b` → two applies)
            // and ensures callPackage inner applies emit their Imports refs.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    extract_value_refs(child, src, source_symbol_index, symbols, refs);
                }
            }
        }
        "select_expression" => {
            // Emit a Calls ref for every select_expression (attribute access).
            // This covers both `pkgs.hello` as a value AND as a function in an apply.
            let name = resolve_call_name(node, src).unwrap_or_else(|| node_text(node, src));
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: node.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
            // Recurse into sub-expressions (but not the attrpath — avoid double-emit)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    extract_value_refs(child, src, source_symbol_index, symbols, refs);
                }
            }
        }
        "attrset_expression" | "rec_attrset_expression" => {
            // Nested attrset — extract its bindings as children
            extract_attrset(node, src, symbols, refs, Some(source_symbol_index), false);
        }
        "let_expression" | "let_attrset_expression" => {
            extract_let(node, src, symbols, refs, Some(source_symbol_index));
        }
        "function_expression" => {
            // Visit formal parameter default values, then the lambda body.
            visit_formal_defaults(node, src, symbols, refs, Some(source_symbol_index));
            if let Some(body) = node.child_by_field_name("body") {
                extract_value_refs(body, src, source_symbol_index, symbols, refs);
            }
        }
        "with_expression" => {
            extract_with(node, src, source_symbol_index, refs);
            if let Some(body) = node.child_by_field_name("body") {
                extract_value_refs(body, src, source_symbol_index, symbols, refs);
            }
        }
        "variable_expression" => {
            // A binding whose RHS is a bare variable (`l = lib`) is an alias.
            // Emit a Reads ref to the aliased name so the head-alias rung can
            // bind a dotted target whose head is this binding (`l.mkOption`).
            if let Some(name) = resolve_call_name(node, src) {
                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::Reads,
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
        _ => {
            // Recurse looking for apply_expression, select_expression, and with_expression
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if is_expr_node(&child) {
                    extract_value_refs(child, src, source_symbol_index, symbols, refs);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared node helpers
// ---------------------------------------------------------------------------

pub(super) fn is_expr_node(node: &Node) -> bool {
    // Named expression nodes to recurse into.
    // `interpolation` is included so that ${ ... } string interpolations are
    // traversed — apply_expression nodes inside them would otherwise be invisible.
    matches!(
        node.kind(),
        "attrset_expression"
            | "rec_attrset_expression"
            | "let_expression"
            | "let_attrset_expression"
            | "with_expression"
            | "apply_expression"
            | "function_expression"
            | "lambda"
            | "if_expression"
            | "assert_expression"
            | "select_expression"
            | "binary_expression"
            | "unary_expression"
            | "parenthesized_expression"
            | "list_expression"
            | "path_expression"
            | "string_expression"
            | "indented_string_expression"
            | "interpolation"
            | "variable_expression"
            | "integer_expression"
            | "float_expression"
            | "uri_expression"
            | "has_attr_expression"
    )
}

pub(super) fn first_child_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

pub(super) fn first_identifier_text(node: &Node, src: &str) -> Option<String> {
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

pub(super) fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}
