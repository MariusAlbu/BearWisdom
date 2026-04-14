//! Swift multiline-string content detection (E19).
//!
//! Detects Swift multiline string literals (`"""…"""`) whose body
//! sniffs as SQL / HTML / JSON / CSS and emits a [`StringDsl`] region.
//!
//! [`StringDsl`]: crate::types::EmbeddedOrigin::StringDsl

use crate::languages::string_dsl;
use crate::types::{EmbeddedOrigin, EmbeddedRegion};
use tree_sitter::{Node, Parser};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_swift::LANGUAGE.into()).is_err() {
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
    // Swift's grammar surfaces multiline strings under `multiline_string_literal`
    // or `raw_string_literal`. Depending on the grammar version any `*string*`
    // kind that starts with `"""` is a candidate.
    let kind = node.kind();
    if kind.contains("string_literal") || kind == "line_string_literal" {
        if let Some(r) = extract_triple_quoted(node, source) {
            regions.push(r);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(&child, source, regions);
    }
}

fn extract_triple_quoted(node: &Node, source: &str) -> Option<EmbeddedRegion> {
    let raw = source.get(node.start_byte()..node.end_byte())?;
    // Accept optional leading `#` count (Swift's extended delimiters) before
    // the triple quote, and the matching suffix.
    let (pre_hashes, rest) = strip_leading_hashes(raw);
    if !rest.starts_with("\"\"\"") || !raw.ends_with("\"\"\"") {
        return None;
    }
    let post_hashes = count_trailing_hashes(raw);
    if post_hashes != pre_hashes {
        return None;
    }
    let inner_start = pre_hashes + 3;
    let inner_end = raw.len() - (3 + post_hashes);
    if inner_end <= inner_start {
        return None;
    }
    let inner = &raw[inner_start..inner_end];
    let lang_id = string_dsl::sniff(inner)?;
    let body_start = node.start_byte() + inner_start;
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

fn strip_leading_hashes(s: &str) -> (usize, &str) {
    let n = s.bytes().take_while(|&b| b == b'#').count();
    (n, &s[n..])
}

fn count_trailing_hashes(s: &str) -> usize {
    s.bytes().rev().take_while(|&b| b == b'#').count()
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
    fn swift_multiline_sql_detected() {
        let src = "let q = \"\"\"\nSELECT id FROM users WHERE active = 1\n\"\"\"\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "sql"),
            "expected sql region, got {regions:?}");
    }

    #[test]
    fn swift_line_string_ignored() {
        let src = "let s = \"hello world short\"\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }
}
