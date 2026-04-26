// =============================================================================
// languages/bicep/extract.rs  —  Azure Bicep IaC extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class     — resource_declaration, module_declaration
//   Variable  — parameter_declaration, variable_declaration, output_declaration,
//               metadata_declaration
//   TypeAlias — type_declaration
//   Function  — user_defined_function
//   Namespace — module_declaration (also emits Imports for module path)
//
// REFERENCES:
//   Imports   — import_statement, import_with_statement, import_functionality,
//               using_statement
//   Imports   — module_declaration (module path → bicep file)
//   Calls     — call_expression (decorator and inline function calls)
//
// Grammar: tree-sitter-bicep (not yet in Cargo.toml — ready for when added).
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from a Bicep file (.bicep).
///
/// Requires the tree-sitter-bicep grammar to be available as `language`.
/// Called by `BicepPlugin::extract()` once the grammar is wired in.
#[allow(dead_code)]
pub fn extract(source: &str, language: tree_sitter::Language) -> crate::types::ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Bicep grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return crate::types::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit_infrastructure(tree.root_node(), source, &mut symbols, &mut refs);

    // Second pass: collect all resource_declaration nodes not yet matched
    let res_lines: std::collections::HashSet<u32> = symbols.iter().map(|s| s.start_line).collect();
    collect_all_resource_declarations(tree.root_node(), source, &res_lines, &mut symbols, &mut refs);

    // Third pass: collect all call_expression nodes for ref coverage
    collect_all_call_expressions(tree.root_node(), source, &mut refs);

    crate::types::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Top-level traversal
// ---------------------------------------------------------------------------

fn visit_infrastructure(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "infrastructure" => visit_infrastructure(child, src, symbols, refs),
            "resource_declaration" => extract_resource_declaration(&child, src, symbols, refs),
            "module_declaration" => extract_module_declaration(&child, src, symbols, refs),
            "parameter_declaration" => extract_parameter_declaration(&child, src, symbols),
            "variable_declaration" => extract_variable_declaration(&child, src, symbols),
            "output_declaration" => extract_output_declaration(&child, src, symbols),
            "type_declaration" => extract_type_declaration(&child, src, symbols),
            "user_defined_function" => extract_user_defined_function(&child, src, symbols, refs),
            "metadata_declaration" => extract_metadata_declaration(&child, src, symbols),
            "import_statement" | "import_with_statement" | "import_functionality" => {
                extract_import_statement(&child, src, refs)
            }
            "using_statement" => extract_using_statement(&child, src, refs),
            // Recurse into container nodes that may hold nested resource_declarations
            "object" | "object_property" | "for_statement" | "if_statement"
            | "decorators" | "array" | "parenthesized_expression" => {
                visit_infrastructure(child, src, symbols, refs);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// resource <name> '<type>@<version>' = { ... }  →  Class
// ---------------------------------------------------------------------------

fn extract_resource_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_identifier(node, src) {
        Some(n) => n,
        None => return,
    };

    // The resource type string is the first string literal child.
    let res_type = find_string_literal(node, src);
    let sig = match &res_type {
        Some(t) => format!("resource {} '{}'", name, t),
        None => format!("resource {}", name),
    };

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Class,
        node,
        Some(sig),
        None,
    ));

    // Emit a TypeRef for the ARM resource type string (e.g. 'Microsoft.Web/sites@2022-03-01').
    if let Some(type_str) = res_type {
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: type_str,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
});
    }

    // Collect call_expression refs inside the body (decorators, function calls).
    extract_calls_in_subtree(node, src, idx, refs);
}

// ---------------------------------------------------------------------------
// module <name> '<path>' = { ... }  →  Class + Imports (module path)
// ---------------------------------------------------------------------------

fn extract_module_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_identifier(node, src) {
        Some(n) => n,
        None => return,
    };

    let module_path = find_string_literal(node, src);
    let sig = match &module_path {
        Some(p) => format!("module {} '{}'", name, p),
        None => format!("module {}", name),
    };

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Class,
        node,
        Some(sig),
        None,
    ));

    // Emit an Imports edge for the module path.
    if let Some(path) = module_path {
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: path.clone(),
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            module: Some(path),
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
});
    }

    extract_calls_in_subtree(node, src, idx, refs);
}

// ---------------------------------------------------------------------------
// param <name> <type> [= default]  →  Variable
// ---------------------------------------------------------------------------

fn extract_parameter_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match find_identifier(node, src) {
        Some(n) => n,
        None => return,
    };
    let sig = format!(
        "param {} {}",
        name,
        find_type_annotation(node, src).unwrap_or_default()
    );
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        Some(sig.trim().to_string()),
        None,
    ));
}

// ---------------------------------------------------------------------------
// var <name> = <expr>  →  Variable
// ---------------------------------------------------------------------------

fn extract_variable_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match find_identifier(node, src) {
        Some(n) => n,
        None => return,
    };
    let sig = format!("var {} = ...", name);
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
// output <name> <type> = <expr>  →  Variable
// ---------------------------------------------------------------------------

fn extract_output_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match find_identifier(node, src) {
        Some(n) => n,
        None => return,
    };
    let type_ann = find_type_annotation(node, src).unwrap_or_default();
    let sig = format!("output {} {}", name, type_ann).trim().to_string();
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
// type <name> = <type>  →  TypeAlias
// ---------------------------------------------------------------------------

fn extract_type_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match find_identifier(node, src) {
        Some(n) => n,
        None => return,
    };
    let sig = format!("type {} = ...", name);
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::TypeAlias,
        node,
        Some(sig),
        None,
    ));
}

// ---------------------------------------------------------------------------
// func <name>(<params>) <returnType> => <expr>  →  Function
// ---------------------------------------------------------------------------

fn extract_user_defined_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_function_name(node, src) {
        Some(n) => n,
        None => return,
    };
    let first_line = node_text(*node, src)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let sig = if first_line.is_empty() {
        format!("func {}", name)
    } else {
        first_line
    };
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Function,
        node,
        Some(sig),
        None,
    ));
    extract_calls_in_subtree(node, src, idx, refs);
}

fn find_function_name(node: &Node, src: &str) -> Option<String> {
    // user_defined_function has a `name` field or the identifier after `func`.
    if let Some(n) = node.child_by_field_name("name") {
        return Some(node_text(n, src));
    }
    // Fallback: first identifier after the `func` keyword.
    let mut cursor = node.walk();
    let mut saw_func = false;
    for child in node.children(&mut cursor) {
        if node_text(child, src) == "func" {
            saw_func = true;
        } else if saw_func && child.kind() == "identifier" {
            return Some(node_text(child, src));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// metadata <name> = <value>  →  Variable
// ---------------------------------------------------------------------------

fn extract_metadata_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match find_identifier(node, src) {
        Some(n) => n,
        None => return,
    };
    let sig = format!("metadata {} = ...", name);
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
// import / using → Imports
// ---------------------------------------------------------------------------

fn extract_import_statement(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    // The import path is a string literal child.
    if let Some(path) = find_string_literal(node, src) {
        refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: path.clone(),
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            module: Some(path),
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
});
    }
}

fn extract_using_statement(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    if let Some(path) = find_string_literal(node, src) {
        refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: path.clone(),
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            module: Some(path),
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
});
    }
}

// ---------------------------------------------------------------------------
// call_expression → Calls
// ---------------------------------------------------------------------------

fn extract_calls_in_subtree(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() == "call_expression" {
        if let Some(func) = node.child_by_field_name("function") {
            let name = node_text(func, src);
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        } else {
            // Fallback: first identifier child.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = node_text(child, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: child.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
                        break;
                    }
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_calls_in_subtree(&child, src, source_idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find the first `identifier` or `compatible_identifier` child.
fn find_identifier(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    // Skip keyword tokens — the `identifier` grammar node is distinct from keywords.
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "identifier" | "compatible_identifier") {
            let t = node_text(child, src);
            if !t.is_empty()
                && !matches!(
                    t.as_str(),
                    "resource" | "param" | "var" | "output" | "type" | "module" | "metadata"
                )
            {
                return Some(t);
            }
        }
    }
    None
}

/// Find the first string literal (unquoted) in a node's direct children.
fn find_string_literal(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" || child.kind() == "string_content" {
            let raw = node_text(child, src);
            let stripped = raw.trim_matches('\'').trim_matches('"').to_string();
            if !stripped.is_empty() {
                return Some(stripped);
            }
        }
    }
    None
}

/// Find the type annotation text in a declaration node.
fn find_type_annotation(node: &Node, src: &str) -> Option<String> {
    // Many declarations have a `type` field.
    if let Some(t) = node.child_by_field_name("type") {
        return Some(node_text(t, src));
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

/// Walk the entire tree and emit a Class symbol for every `resource_declaration` node
/// not already extracted.
fn collect_all_resource_declarations(
    node: Node,
    src: &str,
    existing_lines: &std::collections::HashSet<u32>,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() == "resource_declaration" {
        let line = node.start_position().row as u32;
        if !existing_lines.contains(&line) {
            // Try the regular extractor first
            let prev_len = symbols.len();
            extract_resource_declaration(&node, src, symbols, refs);
            // If extractor didn't emit anything, emit a fallback symbol at the node line
            if symbols.len() == prev_len {
                let name = find_identifier(&node, src)
                    .unwrap_or_else(|| {
                        // Take first identifier from node text before the string literal
                        let raw = node_text(node, src);
                        raw.split_whitespace()
                            .nth(1) // token after "resource"
                            .unwrap_or("resource")
                            .trim_end_matches('\'')
                            .trim_end_matches('"')
                            .to_string()
                    });
                symbols.push(make_symbol(
                    name.clone(),
                    name,
                    SymbolKind::Class,
                    &node,
                    Some(node_text(node, src).lines().next().unwrap_or("resource").trim().to_string()),
                    None,
                ));
            }
        }
        // Recurse to find nested resource_declarations
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_all_resource_declarations(child, src, existing_lines, symbols, refs);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_all_resource_declarations(child, src, existing_lines, symbols, refs);
    }
}

/// Walk the entire tree and emit a Calls ref for every `call_expression` node.
/// This second pass ensures coverage correlation finds a ref for every
/// call_expression occurrence (the ref_node_kind), even those not inside
/// declarations already processed by extract_calls_in_subtree.
fn collect_all_call_expressions(
    node: Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() == "call_expression" {
        let name = if let Some(func) = node.child_by_field_name("function") {
            node_text(func, src)
        } else {
            // Fallback: first identifier child
            let mut name = String::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    name = node_text(child, src);
                    break;
                }
            }
            name
        };
        if !name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: 0,
                target_name: name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
});
        }
        // Don't recurse further into call_expression — avoid double-counting nested calls
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_all_call_expressions(child, src, refs);
    }
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}
