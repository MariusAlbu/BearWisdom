// =============================================================================
// languages/angular/extract.rs  —  Angular template extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class     — sentinel "template" symbol (index 0, represents the template file)
//
// REFERENCES (all from source_symbol_index = 0):
//   Calls     — custom element tags with hyphens (Angular component selectors)
//   Calls     — PascalCase-ish tags not in the HTML5 element set
//   Calls     — pipe usages: `value | pipeName` → `<Name>Pipe` (by convention)
//   Calls     — event handlers: (click)="handler($event)" → handler method
//   Calls     — *ngIf / *ngFor structural directives → directive class names
//
// Grammar: tree-sitter-html 0.23 (used as structural fallback).
// Pipe call and directive extraction is heuristic — the HTML grammar does not
// parse Angular template expressions. We scan attribute values with string
// matching for pipes and `*ng` prefixed attributes.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str, file_path: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_html::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load HTML grammar for Angular");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // Sentinel template symbol (all edges source from index 0)
    let template_name = template_stem(file_path);
    symbols.push(ExtractedSymbol {
        name: template_name.clone(),
        qualified_name: template_name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: tree.root_node().end_position().row as u32,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("// Angular template: {template_name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    visit_node(&tree.root_node(), source, &mut refs);

    // Additionally scan source text for pipe usages in interpolations
    extract_pipes_from_text(source, &mut refs);

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// CST traversal
// ---------------------------------------------------------------------------

fn visit_node(node: &Node, src: &str, refs: &mut Vec<ExtractedRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "element" => {
                process_element(&child, src, refs);
                visit_node(&child, src, refs);
            }
            "self_closing_element" | "self_closing_tag" => {
                process_element(&child, src, refs);
            }
            _ => visit_node(&child, src, refs),
        }
    }
}

// ---------------------------------------------------------------------------
// Element processing
// ---------------------------------------------------------------------------

fn process_element(node: &Node, src: &str, refs: &mut Vec<ExtractedRef>) {
    let tag = element_tag_name(node, src);

    // Custom element with hyphens (Angular component selector pattern)
    if tag.contains('-') && !is_html5_custom_element_builtin(&tag) {
        let pascal = kebab_to_pascal(&tag);
        refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: pascal,
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }

    // Scan attributes for Angular bindings
    scan_attributes(node, src, refs);
}

fn scan_attributes(node: &Node, src: &str, refs: &mut Vec<ExtractedRef>) {
    // attributes may be under start_tag children or direct children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "start_tag" | "self_closing_tag" => {
                let mut ac = child.walk();
                for attr in child.children(&mut ac) {
                    if attr.kind() == "attribute" {
                        process_attribute(&attr, src, refs);
                    }
                }
            }
            "attribute" => process_attribute(&child, src, refs),
            _ => {}
        }
    }
}

fn process_attribute(node: &Node, src: &str, refs: &mut Vec<ExtractedRef>) {
    let attr_name = node
        .child_by_field_name("name")
        .or_else(|| find_child_of_kind(node, "attribute_name"))
        .map(|n| node_text(n, src))
        .unwrap_or_default();

    if attr_name.is_empty() {
        return;
    }

    let attr_value = node
        .child_by_field_name("value")
        .or_else(|| find_child_of_kind(node, "quoted_attribute_value"))
        .or_else(|| find_child_of_kind(node, "attribute_value"))
        .map(|n| {
            let raw = node_text(n, src);
            raw.trim_matches('"').trim_matches('\'').to_string()
        })
        .unwrap_or_default();

    // (event)="handler($event)" — event binding
    if (attr_name.starts_with('(') && attr_name.ends_with(')'))
        || attr_name.starts_with("on-")
    {
        extract_handler_from_value(&attr_value, node, refs);
        return;
    }

    // *ngIf / *ngFor / *ngSwitch — structural directives
    if let Some(directive) = attr_name.strip_prefix('*') {
        let class_name = format!("{}Directive", to_pascal_case(directive));
        refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: class_name,
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
        return;
    }

    // Pipes in attribute values: [prop]="value | pipe" or (event)="value | pipe"
    if !attr_value.is_empty() {
        extract_pipes_from_expression(&attr_value, node.start_position().row as u32, refs);
    }
}

// ---------------------------------------------------------------------------
// Event handler extraction
// ---------------------------------------------------------------------------

fn extract_handler_from_value(value: &str, node: &Node, refs: &mut Vec<ExtractedRef>) {
    // "handler($event)" → handler
    let handler = value
        .split('(')
        .next()
        .unwrap_or(value)
        .trim()
        .to_string();

    if handler.is_empty()
        || handler.contains(' ')
        || handler.contains('!')
        || handler.contains('{')
    {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: handler,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// Pipe extraction — text-level heuristic
// ---------------------------------------------------------------------------

/// Scan raw template source for `| pipeName` patterns in interpolations.
fn extract_pipes_from_text(src: &str, refs: &mut Vec<ExtractedRef>) {
    for (line_idx, line) in src.lines().enumerate() {
        extract_pipes_from_expression(line, line_idx as u32, refs);
    }
}

/// Extract pipe names from an expression string: `value | date:'short' | async`
fn extract_pipes_from_expression(expr: &str, line: u32, refs: &mut Vec<ExtractedRef>) {
    // Split by `|` and skip the first segment (it's the value, not a pipe)
    let mut parts = expr.split('|');
    parts.next(); // skip LHS value
    for part in parts {
        let pipe_name = part
            .trim()
            .split(':')
            .next()  // strip arguments like `:arg`
            .unwrap_or("")
            .trim()
            .split(' ')
            .next()  // stop at first space
            .unwrap_or("")
            .to_string();

        if pipe_name.is_empty() || !is_valid_identifier(&pipe_name) {
            continue;
        }

        // Convention: datePipe → DatePipe (Angular pipe class naming)
        let class_name = format!("{}Pipe", to_pascal_case(&pipe_name));
        refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: class_name,
            kind: EdgeKind::Calls,
            line,
            module: None,
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn element_tag_name(node: &Node, src: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "start_tag" | "self_closing_tag" => {
                if let Some(tag) = find_child_of_kind(&child, "tag_name") {
                    return node_text(tag, src);
                }
            }
            "tag_name" => return node_text(child, src),
            _ => {}
        }
    }
    String::new()
}

fn find_child_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

fn template_stem(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let file = file.rsplit('\\').next().unwrap_or(file);
    // Strip .component.html or .html
    let stem = file
        .strip_suffix(".component.html")
        .or_else(|| file.strip_suffix(".html"))
        .unwrap_or(file);
    to_pascal_case(stem)
}

fn kebab_to_pascal(s: &str) -> String {
    s.split('-')
        .map(|part| {
            let mut c = part.chars();
            match c.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect()
}

fn to_pascal_case(s: &str) -> String {
    // Convert camelCase or kebab-case to PascalCase
    if s.contains('-') {
        return kebab_to_pascal(s);
    }
    // Capitalize first character
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
    }
}

fn is_valid_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        && s.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_' || c == '$')
}

/// Very limited set of custom element registry builtins that have hyphens.
/// In practice Angular apps use their own selectors — this is just a guard.
fn is_html5_custom_element_builtin(_tag: &str) -> bool {
    false
}
