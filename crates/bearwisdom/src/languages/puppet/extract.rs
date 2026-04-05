// =============================================================================
// languages/puppet/extract.rs  —  Puppet infrastructure-as-code extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class    — class_definition (Puppet class)
//   Class    — defined_resource_type (define <name> — callable resource type)
//   Function — function_declaration (Puppet 4+ function)
//   Function — node_definition (matches specific hosts)
//   Variable — resource_declaration (resource instance)
//
// REFERENCES:
//   Imports + Calls — include_statement (include foo::bar)
//   Imports + Calls — require_statement (require foo::bar)
//   Calls           — function_call
//   Calls           — resource_declaration → resource type
//   Inherits        — class_definition with class_inherits
//
// Grammar: tree-sitter-puppet (not yet in Cargo.toml — ready for when added).
// Puppet uses '::' as namespace separator; qualified names preserve it.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from a Puppet manifest (.pp).
///
/// Requires the tree-sitter-puppet grammar to be available as `language`.
/// Called by `PuppetPlugin::extract()` once the grammar is wired in.
#[allow(dead_code)]
pub fn extract(source: &str, language: tree_sitter::Language) -> crate::types::ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Puppet grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return crate::types::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit_manifest(tree.root_node(), source, &mut symbols, &mut refs);

    // Second pass: collect all resource_reference and function_call nodes
    // for ref coverage (catches nodes missed by dispatch_node traversal).
    collect_resource_references(tree.root_node(), source, &mut refs);
    collect_all_function_calls(tree.root_node(), source, &mut refs);

    crate::types::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Manifest traversal
// ---------------------------------------------------------------------------

fn visit_manifest(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch_node(&child, src, symbols, refs, None);
    }
}

fn dispatch_node(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    match node.kind() {
        "class_definition" => extract_class_definition(node, src, symbols, refs),
        "defined_resource_type" => extract_defined_resource_type(node, src, symbols, refs),
        "function_declaration" => extract_function_declaration(node, src, symbols, refs),
        "node_definition" => extract_node_definition(node, src, symbols, refs),
        "resource_declaration" => {
            extract_resource_declaration(node, src, symbols, refs, parent_index)
        }
        "include_statement" => extract_include_or_require(node, src, refs, parent_index),
        "require_statement" => extract_include_or_require(node, src, refs, parent_index),
        "function_call" => extract_function_call(node, src, refs, parent_index),
        _ => {
            // Recurse into block-like containers.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                dispatch_node(&child, src, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// class <name> [inherits <parent>] [($params)] { ... }  →  Class
// ---------------------------------------------------------------------------

fn extract_class_definition(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_class_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let sig = build_class_signature(node, src, &name);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name.clone(),
        SymbolKind::Class,
        node,
        Some(sig),
        None,
    ));

    // Check for `inherits <parent>` — emit Inherits edge.
    if let Some(parent) = find_inherits_name(node, src) {
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: parent,
            kind: EdgeKind::Inherits,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }

    // Recurse into the class body.
    visit_class_body(node, src, symbols, refs, idx);
}

fn find_class_name(node: &Node, src: &str) -> Option<String> {
    // class_definition: `class` keyword, then class identifier.
    let mut cursor = node.walk();
    let mut saw_class_keyword = false;
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Skip the `class` keyword token.
            "class" => { saw_class_keyword = true; }
            "identifier" | "class_identifier" if saw_class_keyword => {
                return Some(node_text(child, src));
            }
            _ => {}
        }
    }
    // Fallback: first identifier/class_identifier child.
    let mut cursor2 = node.walk();
    for child in node.children(&mut cursor2) {
        if matches!(child.kind(), "identifier" | "class_identifier") {
            return Some(node_text(child, src));
        }
    }
    None
}

fn find_inherits_name(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    let mut in_inherits = false;
    for child in node.children(&mut cursor) {
        if child.kind() == "class_inherits" {
            // class_inherits node contains the parent class identifier.
            let mut cc = child.walk();
            for ic in child.children(&mut cc) {
                if matches!(ic.kind(), "identifier" | "class_identifier") {
                    return Some(node_text(ic, src));
                }
            }
        }
        // Handle inline `inherits` keyword followed by identifier.
        if node_text(child, src) == "inherits" {
            in_inherits = true;
        } else if in_inherits && matches!(child.kind(), "identifier" | "class_identifier") {
            return Some(node_text(child, src));
        }
    }
    None
}

fn build_class_signature(node: &Node, src: &str, name: &str) -> String {
    // Take the first line of the class definition as the signature.
    let first_line = node_text(*node, src)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if first_line.is_empty() {
        format!("class {}", name)
    } else {
        first_line
    }
}

fn visit_class_body(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "block" || child.kind() == "body" {
            let mut bc = child.walk();
            for stmt in child.children(&mut bc) {
                dispatch_node(&stmt, src, symbols, refs, Some(parent_index));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// define <name> [($params)] { ... }  →  Class
// ---------------------------------------------------------------------------

fn extract_defined_resource_type(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_define_name(node, src) {
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
        format!("define {}", name)
    } else {
        first_line
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

    visit_class_body(node, src, symbols, refs, idx);
}

fn find_define_name(node: &Node, src: &str) -> Option<String> {
    // defined_resource_type: `define` keyword, then name.
    let mut cursor = node.walk();
    let mut saw_define = false;
    for child in node.children(&mut cursor) {
        if node_text(child, src) == "define" {
            saw_define = true;
        } else if saw_define && matches!(child.kind(), "identifier" | "class_identifier") {
            return Some(node_text(child, src));
        }
    }
    // Fallback via named fields.
    if let Some(n) = node.child_by_field_name("class_identifier") {
        return Some(node_text(n, src));
    }
    if let Some(n) = node.child_by_field_name("identifier") {
        return Some(node_text(n, src));
    }
    None
}

// ---------------------------------------------------------------------------
// function <name>(...): <type> { ... }  →  Function
// ---------------------------------------------------------------------------

fn extract_function_declaration(
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
        format!("function {}", name)
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

    // Recurse into function body.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch_node(&child, src, symbols, refs, Some(idx));
    }
}

fn find_function_name(node: &Node, src: &str) -> Option<String> {
    // function_declaration: `function` keyword, then identifier.
    let mut cursor = node.walk();
    let mut saw_function = false;
    for child in node.children(&mut cursor) {
        if node_text(child, src) == "function" {
            saw_function = true;
        } else if saw_function && child.kind() == "identifier" {
            return Some(node_text(child, src));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// node '<name>' { ... }  →  Function
// ---------------------------------------------------------------------------

fn extract_node_definition(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match find_node_name(node, src) {
        Some(n) => n,
        None => return,
    };

    let sig = format!("node '{}'", name);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Function,
        node,
        Some(sig),
        None,
    ));

    visit_class_body(node, src, symbols, refs, idx);
}

fn find_node_name(node: &Node, src: &str) -> Option<String> {
    // node_definition: `node` keyword, then node_name (string/regex/default/identifier).
    if let Some(nn) = node.child_by_field_name("node_name") {
        return Some(node_text(nn, src).trim_matches('"').trim_matches('\'').to_string());
    }
    // Fallback: first string or identifier after `node`.
    let mut cursor = node.walk();
    let mut saw_node = false;
    for child in node.children(&mut cursor) {
        if node_text(child, src) == "node" {
            saw_node = true;
        } else if saw_node {
            match child.kind() {
                "string" | "identifier" | "default" => {
                    let t = node_text(child, src)
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string();
                    return Some(t);
                }
                _ => {}
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// resource_declaration → Variable + Calls to resource type
// ---------------------------------------------------------------------------

fn extract_resource_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let res_type = match find_resource_type(node, src) {
        Some(t) => t,
        None => return,
    };
    let title = find_resource_title(node, src).unwrap_or_else(|| res_type.clone());

    let name = format!("{}[{}]", res_type, title);
    let sig = format!("{} {{ '{}': ... }}", res_type, title);

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        Some(sig),
        parent_index,
    ));

    // Calls edge to the resource type.
    refs.push(ExtractedRef {
        source_symbol_index: idx,
        target_name: res_type,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

fn find_resource_type(node: &Node, src: &str) -> Option<String> {
    // resource_declaration has a `type` field or the first class_identifier/identifier child.
    if let Some(t) = node.child_by_field_name("type") {
        return Some(node_text(t, src));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "class_identifier" | "identifier") {
            return Some(node_text(child, src));
        }
    }
    None
}

fn find_resource_title(node: &Node, src: &str) -> Option<String> {
    // The title is typically a string literal after the resource type name.
    if let Some(t) = node.child_by_field_name("title") {
        let raw = node_text(t, src);
        return Some(raw.trim_matches('"').trim_matches('\'').to_string());
    }
    // Fallback: first string child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" {
            let raw = node_text(child, src);
            return Some(raw.trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// include / require → Imports + Calls
// ---------------------------------------------------------------------------

fn extract_include_or_require(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(0);

    // Collect all class identifiers from the statement.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "class_identifier" | "identifier") {
            let name = node_text(child, src);
            // Imports edge.
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: name.clone(),
                kind: EdgeKind::Imports,
                line: child.start_position().row as u32,
                module: Some(name.clone()),
                chain: None,
            });
            // Calls edge.
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
}

// ---------------------------------------------------------------------------
// function_call → Calls
// ---------------------------------------------------------------------------

fn extract_function_call(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(0);
    // Emit ref at the function_call node's line (not the identifier child's line)
    // so coverage correlation matches the function_call ref_node_kind.
    let line = node.start_position().row as u32;

    // Try identifier, class_identifier, qualified_name, variable — take the first.
    let name = {
        let mut found = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let k = child.kind();
            if matches!(k, "identifier" | "class_identifier" | "variable" | "qualified_name" | "name") {
                found = node_text(child, src);
                break;
            }
        }
        if found.is_empty() {
            // Fallback: take the first-line text of the node itself (before `(`)
            let raw = node_text(*node, src);
            raw.lines().next().unwrap_or("").split('(').next().unwrap_or("").trim().to_string()
        } else {
            found
        }
    };

    refs.push(ExtractedRef {
        source_symbol_index: source_idx,
        target_name: if name.is_empty() { "fn".to_string() } else { name },
        kind: EdgeKind::Calls,
        line,
        module: None,
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// Helpers
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

/// Walk the entire tree and emit a Calls ref for every `function_call` node.
/// This second pass catches function_calls not visited by dispatch_node.
fn collect_all_function_calls(
    node: Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() == "function_call" {
        let line = node.start_position().row as u32;
        let mut name = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let k = child.kind();
            if matches!(k, "identifier" | "class_identifier" | "variable" | "qualified_name") {
                name = node_text(child, src);
                break;
            }
        }
        if name.is_empty() {
            let raw = node_text(node, src);
            name = raw.lines().next().unwrap_or("").split('(').next().unwrap_or("fn").trim().to_string();
        }
        refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: if name.is_empty() { "fn".to_string() } else { name },
            kind: EdgeKind::Calls,
            line,
            module: None,
            chain: None,
        });
        // Recurse into function_call children to find nested calls
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_all_function_calls(child, src, refs);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_all_function_calls(child, src, refs);
    }
}

/// Walk the entire tree and emit a Calls ref for every `resource_reference` node.
fn collect_resource_references(
    node: Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() == "resource_reference" {
        // resource_reference: Type['title'] — always emit at the node's line
        let line = node.start_position().row as u32;
        let mut name = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let k = child.kind();
            if matches!(k, "class_identifier" | "identifier" | "variable") {
                name = node_text(child, src);
                break;
            }
        }
        if name.is_empty() {
            // Take just the type part (before '[')
            let raw = node_text(node, src);
            name = raw.split('[').next().unwrap_or("").trim().to_string();
        }
        refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: if name.is_empty() { "Resource".to_string() } else { name },
            kind: EdgeKind::TypeRef,
            line,
            module: None,
            chain: None,
        });
        // Still recurse to find nested resource_references
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_resource_references(child, src, refs);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_resource_references(child, src, refs);
    }
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}
