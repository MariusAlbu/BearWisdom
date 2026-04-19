// =============================================================================
// languages/odin/extract.rs  —  Odin extractor (tree-sitter-based)
//
// What we extract
// ---------------
// SYMBOLS:
//   Function   — procedure_declaration / procedure_literal
//   Struct     — struct_declaration / union_declaration
//   Enum       — enum_declaration
//   Variable   — const_declaration / variable_declaration
//   Namespace  — import_declaration (also emits Imports ref)
//
// REFERENCES:
//   Imports    — import_declaration
//   Calls      — call_expression inside procedure bodies
//   TypeRef    — using_statement
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_odin::LANGUAGE.into()).is_err() {
        return ExtractionResult::empty();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::empty(),
    };

    let src = source.as_bytes();
    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    walk_top_level(tree.root_node(), src, &mut symbols, &mut refs);

    ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Top-level walker
// ---------------------------------------------------------------------------

fn walk_top_level(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "source_file" => {
                walk_top_level(child, src, symbols, refs);
            }
            "import_declaration" => {
                extract_import(child, src, symbols, refs);
            }
            "procedure_declaration" => {
                extract_procedure(child, src, symbols, refs, None);
            }
            "struct_declaration" => {
                extract_typed_decl(child, src, symbols, SymbolKind::Struct);
            }
            "enum_declaration" => {
                extract_typed_decl(child, src, symbols, SymbolKind::Enum);
            }
            "union_declaration" => {
                extract_typed_decl(child, src, symbols, SymbolKind::Struct);
            }
            "const_declaration" | "variable_declaration" | "var_declaration" => {
                extract_var_decl(child, src, symbols);
            }
            "const_type_declaration" => {
                // `Name :: Type` — extract all identifiers in the name list
                extract_const_type_decl(child, src, symbols);
            }
            "overloaded_procedure_declaration" => {
                // `Name :: proc { ... }` overload group
                extract_typed_decl(child, src, symbols, SymbolKind::Function);
            }
            "using_statement" => {
                extract_using(child, src, symbols.len(), refs);
            }
            _ => {
                // Recurse in case declarations are wrapped (e.g. foreign blocks)
                walk_top_level(child, src, symbols, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Import declaration
// ---------------------------------------------------------------------------

fn extract_import(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // Grammar: import_declaration → (alias identifier?) (path string_literal)
    // The path string contains quotes we strip.
    let alias = node
        .child_by_field_name("alias")
        .map(|n| node_text(n, src))
        .or_else(|| find_first_identifier(node, src));

    let path = node
        .child_by_field_name("path")
        .map(|n| node_text(n, src))
        .or_else(|| find_string_content(node, src));

    // Derive a short name from path if no alias
    let name = alias.clone().or_else(|| {
        path.as_ref().map(|p| {
            p.trim_matches('"')
                .rsplit('/')
                .next()
                .unwrap_or(p.trim_matches('"'))
                .to_string()
        })
    });

    let name = match name {
        Some(n) if !n.is_empty() => n,
        _ => return,
    };

    let sym_idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("import \"{name}\"")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    let target = path
        .as_ref()
        .map(|p| p.trim_matches('"').to_string())
        .unwrap_or_else(|| name.clone());

    refs.push(ExtractedRef {
        source_symbol_index: sym_idx,
        target_name: target,
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
        byte_offset: 0,
    });
}

// ---------------------------------------------------------------------------
// Procedure declaration
// ---------------------------------------------------------------------------

fn extract_procedure(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .or_else(|| find_first_identifier(node, src))
        .unwrap_or_else(|| format!("<anon_proc_{}>", node.start_position().row + 1));

    if name.is_empty() {
        return;
    }

    let vis = if name.starts_with('_') {
        Visibility::Private
    } else {
        Visibility::Public
    };

    let sig = build_proc_signature(node, src, &name);
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name,
        kind: SymbolKind::Function,
        visibility: Some(vis),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: 0,
        end_col: 0,
        signature: Some(sig),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    // Extract calls from the procedure body
    extract_calls_in_subtree(node, src, idx, refs);
}

// ---------------------------------------------------------------------------
// Struct / Enum / Union declarations
// ---------------------------------------------------------------------------

fn extract_typed_decl(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    kind: SymbolKind,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .or_else(|| find_first_identifier(node, src))
        .unwrap_or_default();

    if name.is_empty() {
        return;
    }

    let vis = if name.starts_with('_') {
        Visibility::Private
    } else {
        Visibility::Public
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name,
        kind,
        visibility: Some(vis),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
}

// ---------------------------------------------------------------------------
// Variable / constant declarations
// ---------------------------------------------------------------------------

fn extract_var_decl(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
) {
    // Collect all identifier children (for grouped: `a, b :: value`)
    let names = collect_identifiers(node, src);
    if names.is_empty() {
        return;
    }

    for name in names {
        let vis = if name.starts_with('_') {
            Visibility::Private
        } else {
            Visibility::Public
        };
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name,
            kind: SymbolKind::Variable,
            visibility: Some(vis),
            start_line: node.start_position().row as u32,
            end_line: node.end_position().row as u32,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        });
    }
}

/// `Name :: Type` — for type alias / struct / enum / union used as constant type decl
fn extract_const_type_decl(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let names = collect_identifiers(node, src);
    for name in names {
        let vis = if name.starts_with('_') {
            Visibility::Private
        } else {
            Visibility::Public
        };
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name,
            kind: SymbolKind::TypeAlias,
            visibility: Some(vis),
            start_line: node.start_position().row as u32,
            end_line: node.end_position().row as u32,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        });
    }
}

/// Collect all top-level identifier children from a node.
/// Stops at `::`, `:=`, or `=` to avoid capturing RHS identifiers.
fn collect_identifiers(node: Node, src: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "::" | ":=" | "=" => break,
                "identifier" => {
                    let t = node_text(child, src);
                    if !t.is_empty() {
                        names.push(t);
                    }
                }
                _ => {}
            }
        }
    }
    names
}

// ---------------------------------------------------------------------------
// using_statement → TypeRef
// ---------------------------------------------------------------------------

fn extract_using(
    node: Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // `using pkg` or `using pkg.Type` — grab the identifier after "using"
    if let Some(id) = find_first_identifier(node, src) {
        if !id.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: id,
                kind: EdgeKind::TypeRef,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Call extraction in procedure bodies
// ---------------------------------------------------------------------------

fn extract_calls_in_subtree(
    node: Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            // The called expression is usually the first child
            let target = child
                .child_by_field_name("function")
                .or_else(|| child.child(0))
                .map(|n| node_text(n, src))
                .unwrap_or_default();

            // Strip member access: `pkg.Proc` → `Proc`
            let target = target.rsplit('.').next().unwrap_or(&target).to_string();

            if !target.is_empty() && target != "(" {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: target,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
            // Recurse into arguments
            extract_calls_in_subtree(child, src, source_symbol_index, refs);
        } else {
            extract_calls_in_subtree(child, src, source_symbol_index, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text(node: Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").to_string()
}

fn find_first_identifier(node: Node, src: &[u8]) -> Option<String> {
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

fn find_string_content(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "interpreted_string_literal"
            || child.kind() == "string_literal"
            || child.kind() == "string"
        {
            return Some(node_text(child, src));
        }
    }
    None
}

fn build_proc_signature(node: Node, src: &[u8], name: &str) -> String {
    // Look for a procedure type or parameters child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "procedure_type" | "parameters" => {
                return format!("{name}{}", node_text(child, src));
            }
            _ => {}
        }
    }
    format!("{name}()")
}
