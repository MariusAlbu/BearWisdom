//! C# raw/verbatim string content detection.
//!
//! Walks the C# AST for `verbatim_string_literal` (`@"…"`) and
//! `raw_string_literal` (`"""…"""`) nodes whose body content sniffs as
//! a recognisable DSL (SQL, HTML, JSON, CSS) and emits an
//! [`EmbeddedRegion`] with `origin = StringDsl`.
//!
//! Interpolated and plain `"..."` literals are skipped — they usually
//! contain prose, names, or short tokens, and the false-positive risk
//! is too high.

use crate::languages::string_dsl;
use crate::types::{EmbeddedOrigin, EmbeddedRegion};
use tree_sitter::{Node, Parser};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).is_err() {
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
    match node.kind() {
        "verbatim_string_literal" => {
            if let Some(r) = extract_verbatim(node, source) {
                regions.push(r);
            }
        }
        "raw_string_literal" => {
            if let Some(r) = extract_raw(node, source) {
                regions.push(r);
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(&child, source, regions);
    }
}

/// `@"..."` — strip the leading `@"` and trailing `"`. Escaped `""`
/// unescapes to a single `"`.
fn extract_verbatim(node: &Node, source: &str) -> Option<EmbeddedRegion> {
    let raw = source.get(node.start_byte()..node.end_byte())?;
    if !raw.starts_with("@\"") || !raw.ends_with('"') || raw.len() < 3 {
        return None;
    }
    let inner = &raw[2..raw.len() - 1];
    let body = inner.replace("\"\"", "\"");
    let lang_id = string_dsl::sniff(&body)?;
    let body_start = node.start_byte() + 2;
    let (line_offset, col_offset) = byte_to_line_col(source, body_start);
    Some(EmbeddedRegion {
        language_id: lang_id.to_string(),
        text: body,
        line_offset,
        col_offset,
        origin: EmbeddedOrigin::StringDsl,
        holes: Vec::new(),
        strip_scope_prefix: None,
    })
}

/// `"""…"""` — strip matched triple-quote fences (C# 11 raw strings
/// support any number of `"` ≥ 3 as the fence).
fn extract_raw(node: &Node, source: &str) -> Option<EmbeddedRegion> {
    let raw = source.get(node.start_byte()..node.end_byte())?;
    let (open_len, close_len) = count_outer_quotes(raw)?;
    if raw.len() < open_len + close_len {
        return None;
    }
    let inner = &raw[open_len..raw.len() - close_len];
    // Raw strings allow a leading newline + indent — trim nothing so
    // line attribution stays accurate; the sniffer trims internally.
    let lang_id = string_dsl::sniff(inner)?;
    let body_start = node.start_byte() + open_len;
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

fn count_outer_quotes(raw: &str) -> Option<(usize, usize)> {
    let bytes = raw.as_bytes();
    let mut open = 0usize;
    while open < bytes.len() && bytes[open] == b'"' {
        open += 1;
    }
    if open < 3 {
        return None;
    }
    let mut close = 0usize;
    while close < bytes.len() && bytes[bytes.len() - 1 - close] == b'"' {
        close += 1;
    }
    if close < 3 {
        return None;
    }
    Some((open, close))
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
    fn verbatim_sql_detected() {
        let src = r#"
class Q {
    string query = @"SELECT id, name FROM users WHERE active = 1";
}
"#;
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "sql");
        assert_eq!(regions[0].origin, EmbeddedOrigin::StringDsl);
        assert!(regions[0].text.contains("SELECT"));
    }

    #[test]
    fn raw_triple_quoted_sql_detected() {
        let src = r#"
class Q {
    string query = """
        SELECT id FROM users WHERE id = 1
        """;
}
"#;
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "sql"));
    }

    #[test]
    fn short_strings_ignored() {
        let src = r#"
class Q {
    string s = @"hello";
    string t = @"UPDATE";
}
"#;
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn verbatim_html_detected() {
        let src = r#"
class X {
    string html = @"<div class=""hdr""><p>Hello</p></div>";
}
"#;
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "html"));
    }

    #[test]
    fn verbatim_prose_ignored() {
        let src = r#"
class X {
    string msg = @"This is just some prose that happens to be long enough to maybe fool the sniffer";
}
"#;
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }
}
