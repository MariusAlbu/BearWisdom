//! Java text-block content detection.
//!
//! Walks the Java AST for `text_block` nodes (Java 15+ `"""…"""`) and
//! emits a [`StringDsl`] region when the body sniffs as SQL / HTML /
//! JSON / CSS. Plain `"…"` strings are skipped — too noisy.
//!
//! [`StringDsl`]: crate::types::EmbeddedOrigin::StringDsl

use crate::languages::string_dsl;
use crate::types::{EmbeddedOrigin, EmbeddedRegion};
use tree_sitter::{Node, Parser};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_java::LANGUAGE.into()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut regions = Vec::new();
    walk(&tree.root_node(), source, &mut regions);
    regions
}

fn walk(node: &Node, source: &str, regions: &mut Vec<EmbeddedRegion>) {
    // tree-sitter-java parses Java 15+ text blocks as `string_literal`
    // nodes containing a `multiline_string_fragment` child. The outer
    // text starts with `"""`.
    if node.kind() == "string_literal" {
        if let Some(r) = extract_text_block(node, source) {
            regions.push(r);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(&child, source, regions);
    }
}

fn extract_text_block(node: &Node, source: &str) -> Option<EmbeddedRegion> {
    let raw = source.get(node.start_byte()..node.end_byte())?;
    if !raw.starts_with("\"\"\"") || !raw.ends_with("\"\"\"") || raw.len() < 6 {
        return None;
    }
    let inner = &raw[3..raw.len() - 3];
    let lang_id = string_dsl::sniff(inner)?;
    let body_start = node.start_byte() + 3;
    let (line_offset, col_offset) = byte_to_line_col(source, body_start);
    Some(EmbeddedRegion {
        language_id: lang_id.to_string(),
        text: inner.to_string(),
        line_offset,
        col_offset,
        origin: EmbeddedOrigin::StringDsl,
        holes: Vec::new(),
        strip_scope_prefix: None,
    })
}

fn byte_to_line_col(source: &str, byte: usize) -> (u32, u32) {
    let prefix = &source[..byte.min(source.len())];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
    let col = match prefix.rfind('\n') {
        Some(nl) => (byte - nl - 1) as u32,
        None => byte as u32,
    };
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_block_sql_detected() {
        let src = "class Q {\n  String q = \"\"\"\n    SELECT id FROM users WHERE id = ?\n    \"\"\";\n}\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "sql");
    }

    #[test]
    fn text_block_json_detected() {
        let src = "class Q {\n  String payload = \"\"\"\n    {\"name\": \"alice\", \"age\": 30}\n    \"\"\";\n}\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "json"));
    }

    #[test]
    fn plain_string_literal_ignored() {
        let src = "class X { String q = \"SELECT * FROM t\"; }";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn short_text_block_ignored() {
        let src = "class X { String x = \"\"\"\nhi\n\"\"\"; }";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }
}
