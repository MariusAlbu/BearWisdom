//! HTML host-level extraction.
//!
//! Emits:
//!   * A file-stem `Class` symbol so the host file is navigable.
//!   * One `Field` symbol per element carrying an `id="…"` attribute
//!     (navigable anchor).
//!   * One `Imports` ref per `<script src="…">` tag, consumed downstream
//!     by the demand-driven script-tag indexer stage.

use crate::languages::common::extract_script_refs;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    // Auto-generated HTML files (Robot Framework reports, JavaDoc,
    // pydoc, etc.) embed thousands of script-block calls into legacy
    // libraries (jQuery `merge`, `pushStack`, `trigger`) and contribute
    // tens of thousands of irresolvable refs that aren't first-party
    // code. Detect the `<meta name="Generator" content="…">` marker
    // these tools emit and skip extraction entirely. The first-party
    // file-stem symbol is also dropped because navigating to a
    // generated artifact has no value.
    if looks_generated_html(source) {
        return ExtractionResult::empty();
    }

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

    // Script-src refs are collected via a byte-level scan that tolerates
    // templated/`@`/`{{`-laden source and doesn't require tree-sitter-html
    // to accept every unusual syntax.
    let mut refs: Vec<ExtractedRef> = extract_script_refs(source)
        .into_iter()
        .map(|sr| ExtractedRef {
            source_symbol_index: host_index,
            target_name: sr.url.clone(),
            kind: EdgeKind::Imports,
            line: sr.line,
            module: Some(sr.url),
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
})
        .collect();

    let language: tree_sitter::Language = tree_sitter_html::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return ExtractionResult {
            symbols,
            refs,
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors: true,
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
        };
    }

    let Some(tree) = parser.parse(source, None) else {
        return ExtractionResult {
            symbols,
            refs,
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors: true,
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
        };
    };

    collect_anchors(&tree.root_node(), source, &file_name, host_index, &mut symbols);
    let _ = &mut refs; // silence unused-mut lint when no refs were added

    ExtractionResult {
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: tree.root_node().has_error(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
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

/// Detect HTML files emitted by documentation/report generators.
///
/// These files embed thousands of script-block calls into bundled
/// legacy libraries (jQuery, etc.) that contribute massive noise to
/// the unresolved-refs metric without representing first-party code.
/// We scan only the first ~16 KB to keep the check cheap; generator
/// `<meta>` tags always live in the document head.
///
/// Public to the html module so `embedded_regions()` can also bail
/// before extracting `<script>`/`<style>` content from generated docs.
pub(super) fn looks_generated_html(source: &str) -> bool {
    let head = &source.as_bytes()[..source.len().min(16 * 1024)];
    let head_str = match std::str::from_utf8(head) {
        Ok(s) => s,
        Err(_) => return false,
    };
    // Common form: `<meta ... name="Generator" content="...">` (HTML 4)
    // or `<meta name="generator" content="...">` (HTML 5). Capitalization
    // of `name`/`content` and quote style vary; do a small set of
    // tolerant matches rather than parsing.
    let lower = head_str.to_ascii_lowercase();
    if !lower.contains("<meta") {
        return false;
    }
    // Collapse whitespace runs so attribute order doesn't matter.
    let collapsed: String = lower
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    collapsed.contains("name=\"generator\"")
        || collapsed.contains("name='generator'")
        || collapsed.contains("name=generator")
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod tests;
