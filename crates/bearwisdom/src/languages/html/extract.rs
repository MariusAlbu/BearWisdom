//! HTML host-level extraction.
//!
//! Emits a file-stem `Class` symbol plus one `Field` symbol per
//! element carrying an `id="…"` attribute (navigable anchor).

use crate::types::{ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();

    let file_name = file_stem(file_path);
    symbols.push(ExtractedSymbol {
        name: file_name.clone(),
        qualified_name: file_name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
    let host_index = 0usize;

    let language: tree_sitter::Language = tree_sitter_html::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return ExtractionResult {
            symbols,
            refs: Vec::new(),
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors: true,
        };
    }

    let Some(tree) = parser.parse(source, None) else {
        return ExtractionResult {
            symbols,
            refs: Vec::new(),
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors: true,
        };
    };

    collect_anchors(&tree.root_node(), source, &file_name, host_index, &mut symbols);

    ExtractionResult {
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: tree.root_node().has_error(),
    }
}

fn collect_anchors(
    node: &Node,
    source: &str,
    file_name: &str,
    host_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if matches!(kind, "element" | "self_closing_element") {
            if let Some(id) = element_id(&child, source) {
                symbols.push(ExtractedSymbol {
                    name: id.clone(),
                    qualified_name: format!("{file_name}.{id}"),
                    kind: SymbolKind::Field,
                    visibility: Some(Visibility::Public),
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: 0,
                    end_col: 0,
                    signature: None,
                    doc_comment: None,
                    scope_path: Some(file_name.to_string()),
                    parent_index: Some(host_index),
                });
            }
        }
        collect_anchors(&child, source, file_name, host_index, symbols);
    }
}

/// Read `id="…"` from the start tag of an `element` / `self_closing_element`
/// node. Returns `None` if absent or empty.
fn element_id(element: &Node, source: &str) -> Option<String> {
    let mut cursor = element.walk();
    for child in element.children(&mut cursor) {
        if child.kind() == "start_tag" || child.kind() == "self_closing_tag" {
            return read_id_attribute(&child, source);
        }
    }
    None
}

fn read_id_attribute(start_tag: &Node, source: &str) -> Option<String> {
    let mut cursor = start_tag.walk();
    for child in start_tag.children(&mut cursor) {
        if child.kind() != "attribute" {
            continue;
        }
        let mut got_id = false;
        let mut value: Option<String> = None;
        let mut ac = child.walk();
        for attr_child in child.children(&mut ac) {
            match attr_child.kind() {
                "attribute_name" => {
                    let n = source.get(attr_child.start_byte()..attr_child.end_byte())?;
                    if n.eq_ignore_ascii_case("id") {
                        got_id = true;
                    }
                }
                "quoted_attribute_value" => {
                    let mut vc = attr_child.walk();
                    for v_child in attr_child.children(&mut vc) {
                        if v_child.kind() == "attribute_value" {
                            value = source
                                .get(v_child.start_byte()..v_child.end_byte())
                                .map(str::to_string);
                        }
                    }
                }
                "attribute_value" => {
                    value = source
                        .get(attr_child.start_byte()..attr_child.end_byte())
                        .map(str::to_string);
                }
                _ => {}
            }
        }
        if got_id {
            return value.filter(|v| !v.is_empty());
        }
    }
    None
}

fn file_stem(file_path: &str) -> String {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    stem.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_host_symbol_named_after_stem() {
        let r = extract("<html></html>", "docs/index.html");
        assert_eq!(r.symbols[0].name, "index");
        assert_eq!(r.symbols[0].kind, SymbolKind::Class);
    }

    #[test]
    fn element_id_becomes_anchor_symbol() {
        let src = r#"<html><body><section id="intro">Hi</section><div id="footer"></div></body></html>"#;
        let r = extract(src, "page.html");
        let ids: Vec<&str> = r
            .symbols
            .iter()
            .skip(1)
            .map(|s| s.name.as_str())
            .collect();
        assert!(ids.contains(&"intro"));
        assert!(ids.contains(&"footer"));
    }

    #[test]
    fn element_without_id_not_anchored() {
        let src = "<html><div>text</div></html>";
        let r = extract(src, "page.html");
        assert_eq!(r.symbols.len(), 1);
    }
}
