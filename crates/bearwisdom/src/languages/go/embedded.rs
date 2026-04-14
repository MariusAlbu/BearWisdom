//! Go raw-string content detection.
//!
//! Walks the Go AST for `raw_string_literal` nodes (`` `…` ``) and
//! emits a [`StringDsl`] region when the body sniffs as SQL / HTML /
//! JSON / CSS. Plain `"…"` (interpreted) strings are skipped — too
//! noisy (function names, URLs, format strings live there).
//!
//! [`StringDsl`]: crate::types::EmbeddedOrigin::StringDsl

use crate::languages::string_dsl;
use crate::types::{EmbeddedOrigin, EmbeddedRegion};
use tree_sitter::{Node, Parser};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_go::LANGUAGE.into()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut regions = Vec::new();
    walk(&tree.root_node(), source, &mut regions);
    regions.extend(detect_go_generate(source));
    regions
}

/// Find `//go:generate <command>` directives (no space between `//`
/// and `go:generate` — Go tooling is strict about that) and emit the
/// command as a bash region with [`EmbeddedOrigin::BuildToolShell`].
fn detect_go_generate(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    for (line_no, line) in source.lines().enumerate() {
        let Some(rest) = line.strip_prefix("//go:generate ") else {
            continue;
        };
        let cmd = rest.trim();
        if cmd.is_empty() {
            continue;
        }
        regions.push(EmbeddedRegion {
            language_id: "bash".into(),
            text: format!("{cmd}\n"),
            line_offset: line_no as u32,
            col_offset: 0,
            origin: EmbeddedOrigin::BuildToolShell,
            holes: Vec::new(),
            strip_scope_prefix: None,
        });
    }
    regions
}

fn walk(node: &Node, source: &str, regions: &mut Vec<EmbeddedRegion>) {
    if node.kind() == "raw_string_literal" {
        if let Some(r) = extract_raw(node, source) {
            regions.push(r);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(&child, source, regions);
    }
}

fn extract_raw(node: &Node, source: &str) -> Option<EmbeddedRegion> {
    let raw = source.get(node.start_byte()..node.end_byte())?;
    if !raw.starts_with('`') || !raw.ends_with('`') || raw.len() < 2 {
        return None;
    }
    let inner = &raw[1..raw.len() - 1];
    let lang_id = string_dsl::sniff(inner)?;
    let body_start = node.start_byte() + 1;
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
    fn backtick_sql_detected() {
        let src = "package p\n\nvar q = `SELECT id, name FROM users WHERE active = true`\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "sql");
    }

    #[test]
    fn backtick_json_detected() {
        let src = "package p\n\nvar j = `{\"name\": \"alice\", \"age\": 30}`\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "json"));
    }

    #[test]
    fn backtick_html_detected() {
        let src = "package p\n\nvar h = `<div class=\"foo\"><p>Hello</p></div>`\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "html"));
    }

    #[test]
    fn short_backtick_string_ignored() {
        let src = "package p\n\nvar s = `hi`\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn interpreted_string_ignored() {
        let src = "package p\n\nvar q = \"SELECT * FROM users WHERE id = 1\"\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn go_generate_directive_emits_build_tool_shell() {
        let src = "package p\n\n//go:generate stringer -type=Pill\n//go:generate protoc --go_out=. api.proto\nfunc f() {}\n";
        let regions = detect_regions(src);
        let bash: Vec<_> = regions.iter().filter(|r| r.origin == EmbeddedOrigin::BuildToolShell).collect();
        assert_eq!(bash.len(), 2);
        assert_eq!(bash[0].language_id, "bash");
        assert!(bash[0].text.contains("stringer"));
        assert!(bash[1].text.contains("protoc"));
    }

    #[test]
    fn go_generate_with_space_not_recognized() {
        // `// go:generate` (with a space) is NOT treated as a generate
        // directive by Go tooling — Mark it skipped.
        let src = "package p\n\n// go:generate echo hi\n";
        let regions = detect_regions(src);
        assert!(regions.iter().all(|r| r.origin != EmbeddedOrigin::BuildToolShell));
    }
}
