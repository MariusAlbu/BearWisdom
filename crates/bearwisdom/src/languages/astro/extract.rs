// =============================================================================
// languages/astro/extract.rs  —  Astro page/component extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class     — the component/page itself (name = filename stem, line 0)
//
// REFERENCES (source_symbol_index = 0, the component symbol):
//   Calls     — PascalCase tags (island component usages)
//   Calls     — kebab-case custom element tags (normalised to PascalCase)
//
// Grammar: tree-sitter-html 0.23 (structural fallback).
// The frontmatter `---` block is not parsed here; the JS/TS extractor handles
// it separately when the indexer processes injection points.
//
// Astro-specific notes:
// - `client:load`, `client:idle`, `client:visible`, `client:only`, `client:media`
//   are hydration directives. The component is already captured via the PascalCase
//   tag check; we do not emit extra edges for the directive itself.
// - `<slot />` and `<Fragment>` are Astro built-ins — not component calls.
// - The frontmatter (`---` delimited block) may appear as raw_text; its JS import
//   declarations are the canonical source of component references, but those are
//   processed by the JS extractor on the injection point.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

/// Built-in Astro/HTML tag names that should not be treated as component calls.
const BUILTIN_TAGS: &[&str] = &[
    "DOCTYPE", "CDATA",
    // Astro built-ins
    "Fragment", "slot",
];

pub fn extract(source: &str, file_path: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_html::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load HTML grammar for Astro");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // Infer component name from filename stem (e.g. BlogPost.astro → BlogPost)
    let component_name = file_stem(file_path);
    symbols.push(ExtractedSymbol {
        name: component_name.clone(),
        qualified_name: component_name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: tree.root_node().end_position().row as u32,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("// Astro component: {component_name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

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
                process_element(&child, src, refs);
                visit_document(&child, src, refs);
            }
            "self_closing_element" => {
                process_element(&child, src, refs);
            }
            _ => visit_document(&child, src, refs),
        }
    }
}

// ---------------------------------------------------------------------------
// Element processing
// ---------------------------------------------------------------------------

fn process_element(node: &Node, src: &str, refs: &mut Vec<ExtractedRef>) {
    let tag = element_tag_name(node, src);
    if tag.is_empty() {
        return;
    }

    // Skip known built-ins
    if BUILTIN_TAGS.contains(&tag.as_str()) {
        return;
    }

    // PascalCase tags → component call
    if tag.chars().next().map_or(false, |c| c.is_uppercase()) {
        refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: tag,
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
        });
        return;
    }

    // Kebab-case custom elements → PascalCase component call
    if tag.contains('-') {
        refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: kebab_to_pascal(&tag),
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
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
