// =============================================================================
// languages/svelte/extract.rs  —  Svelte SFC extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class     — the component itself (name = filename stem, line 0)
//
// REFERENCES (source_symbol_index = 0, the component symbol):
//   Calls     — PascalCase tags in the template (component usages)
//   Calls     — kebab-case custom element tags (normalised to PascalCase)
//   Calls     — on:event="handler" / on:event={handler} directives
//   Calls     — {#each expr} / {#if expr} → identifier from the expression
//              when the expression starts with an identifier (the iterable ref)
//
// Grammar: tree-sitter-html 0.23 (structural fallback).
// The <script> block content is not parsed here — the JS/TS extractor handles
// it separately when the indexer processes injection points.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

/// Standard HTML tags that start with an uppercase letter, plus other known
/// non-component names. PascalCase check catches most; this set handles edge cases.
const BUILTIN_HTML_TAGS: &[&str] = &["DOCTYPE", "CDATA"];

/// Svelte built-in control-flow / logic block keywords — not component calls.
const SVELTE_KEYWORDS: &[&str] = &["if", "else", "each", "await", "then", "catch", "key"];

pub fn extract(source: &str, file_path: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_html::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load HTML grammar for Svelte");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // Infer component name from filename stem. Kebab-case filenames are
    // converted to PascalCase: `menu-option.svelte` → `MenuOption`. This
    // matches what users actually write in import statements
    // (`import MenuOption from './menu-option.svelte'`) — without the
    // conversion, the symbol's name field is `menu-option` and the resolver
    // can't match the imported alias against any indexed symbol in the file.
    let stem = file_stem(file_path);
    let component_name = if stem.contains('-') {
        kebab_to_pascal(&stem)
    } else {
        stem.clone()
    };
    symbols.push(ExtractedSymbol {
        name: component_name.clone(),
        qualified_name: component_name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: tree.root_node().end_position().row as u32,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("// Svelte component: {component_name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    // Walk for component usages, event handlers, and block references.
    let root = tree.root_node();
    visit_document(&root, source, &mut refs);

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Document traversal
// ---------------------------------------------------------------------------

fn visit_document(node: &Node, src: &str, refs: &mut Vec<ExtractedRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "element" => {
                // Recurse into all top-level elements (template body, etc.)
                process_element(&child, src, refs);
                visit_template(&child, src, refs);
            }
            "raw_text" => {
                // Svelte control-flow blocks ({#if}, {#each}) appear as raw_text
                // inside the HTML parse tree. Mine them for identifiers.
                extract_svelte_blocks(&child, src, refs);
            }
            _ => visit_document(&child, src, refs),
        }
    }
}

fn visit_template(node: &Node, src: &str, refs: &mut Vec<ExtractedRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "element" => {
                process_element(&child, src, refs);
                visit_template(&child, src, refs);
            }
            "self_closing_element" => {
                process_element(&child, src, refs);
            }
            "raw_text" => {
                extract_svelte_blocks(&child, src, refs);
            }
            _ => visit_template(&child, src, refs),
        }
    }
}

// ---------------------------------------------------------------------------
// Element processing
// ---------------------------------------------------------------------------

fn process_element(node: &Node, src: &str, refs: &mut Vec<ExtractedRef>) {
    let tag = element_tag_name(node, src);

    // Component usages: PascalCase or kebab-case with hyphens
    if let Some(component_name) = as_component_name(&tag) {
        refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: component_name,
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
});
    }

    // Scan attributes for on:event directives
    extract_event_handlers(node, src, refs);
}

// ---------------------------------------------------------------------------
// on:event directive extraction
// ---------------------------------------------------------------------------

fn extract_event_handlers(node: &Node, src: &str, refs: &mut Vec<ExtractedRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "start_tag" | "self_closing_tag" => {
                let mut ac = child.walk();
                for attr in child.children(&mut ac) {
                    if attr.kind() == "attribute" {
                        try_extract_on_handler(&attr, src, refs);
                    }
                }
            }
            _ => {}
        }
    }
}

fn try_extract_on_handler(node: &Node, src: &str, refs: &mut Vec<ExtractedRef>) {
    let attr_name = node
        .child_by_field_name("name")
        .or_else(|| find_child_of_kind(node, "attribute_name"))
        .map(|n| node_text(n, src))
        .unwrap_or_default();

    // Svelte event directive: on:click, on:submit, etc.
    if !attr_name.starts_with("on:") {
        return;
    }

    let handler = node
        .child_by_field_name("value")
        .or_else(|| find_child_of_kind(node, "quoted_attribute_value"))
        .or_else(|| find_child_of_kind(node, "attribute_value"))
        .map(|n| {
            let raw = node_text(n, src);
            // Strip outer quotes and curly braces: "{handler}" → "handler"
            raw.trim_matches('"')
                .trim_matches('\'')
                .trim_matches('{')
                .trim_matches('}')
                .to_string()
        })
        .unwrap_or_default();

    // Strip argument expressions: "handler($event)" → "handler"
    let handler = handler
        .split('(')
        .next()
        .unwrap_or(&handler)
        .trim()
        .to_string();

    if handler.is_empty() || handler.contains(' ') || handler.contains('{') {
        return; // inline expression, not a named handler
    }

    refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: handler,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
        byte_offset: 0,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Svelte control-flow block extraction ({#each items ...}, {#if condition})
// ---------------------------------------------------------------------------

/// Scan raw_text nodes for Svelte `{#each <expr>}` / `{#if <expr>}` constructs.
/// Emits a Calls edge for the leading identifier of the expression, if present.
fn extract_svelte_blocks(node: &Node, src: &str, refs: &mut Vec<ExtractedRef>) {
    let text = node_text(*node, src);
    // Look for patterns like: {#each items as ...} or {#if condition}
    let patterns: &[&str] = &["{#each ", "{#if ", "{#await "];
    for pat in patterns {
        if let Some(rest) = text.find(pat).map(|i| &text[i + pat.len()..]) {
            // Take up to the first space, '.', '(', '{', '}', or '#'
            let ident: String = rest
                .chars()
                .take_while(|&c| c.is_alphanumeric() || c == '_')
                .collect();
            if !ident.is_empty() && !SVELTE_KEYWORDS.contains(&ident.as_str()) {
                refs.push(ExtractedRef {
                    source_symbol_index: 0,
                    target_name: ident,
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
    }
}

// ---------------------------------------------------------------------------
// Component name detection
// ---------------------------------------------------------------------------

fn as_component_name(tag: &str) -> Option<String> {
    if tag.is_empty() {
        return None;
    }
    if BUILTIN_HTML_TAGS.contains(&tag) {
        return None;
    }
    // PascalCase: starts with uppercase
    if tag.chars().next().map_or(false, |c| c.is_uppercase()) {
        return Some(tag.to_string());
    }
    // Kebab-case with at least one hyphen → PascalCase
    if tag.contains('-') {
        return Some(kebab_to_pascal(tag));
    }
    None
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

fn file_stem(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let file = file.rsplit('\\').next().unwrap_or(file);
    if let Some(dot) = file.rfind('.') {
        file[..dot].to_string()
    } else {
        file.to_string()
    }
}
