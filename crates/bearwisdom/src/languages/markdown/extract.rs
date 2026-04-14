//! Markdown host-level extraction.
//!
//! The Markdown plugin emits:
//!
//!   * Heading symbols — `# Title`, `## Sub`, … each becomes a
//!     `SymbolKind::Field` named after the heading text. `file_symbols`
//!     on a README then shows a real outline.
//!
//!   * Fenced-block anchor symbols — one synthetic `SymbolKind::Class`
//!     per fenced code block, named `<lang>#<index>` under the file's
//!     dotted path (e.g. `README.ts#1`). Lets users query "all fenced
//!     TypeScript examples in docs/".
//!
//!   * Link refs — `[text](./path/to/file.md)` with a relative path
//!     becomes an `Imports` ref targeting the file stem. Image refs
//!     `![alt](path)` are treated the same.
//!
//! Frontmatter and fenced-block CONTENTS are not handled here — they
//! go through the embedded-region dispatch path in `embedded.rs`.

use super::fenced;
use super::info_string;
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // File-level symbol so child symbols (headings, fence anchors) have
    // a parent to nest under and the resolver can match relative-link
    // references.
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
    let host_index: usize = 0;

    // Heading symbols — ATX style (`#`, `##`, ...) only. Setext
    // underlining (`===` below a line) is rare in modern Markdown docs
    // and the scanner stays simpler without it.
    let bytes = source.as_bytes();
    let mut line_no: u32 = 0;
    let mut ls = 0usize;
    while ls < bytes.len() {
        let le = bytes[ls..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| ls + p)
            .unwrap_or(bytes.len());
        let line = &bytes[ls..le];
        if let Some((level, text)) = parse_atx_heading(line) {
            symbols.push(ExtractedSymbol {
                name: text.clone(),
                qualified_name: format!("{file_name}.{}", slugify(&text)),
                kind: SymbolKind::Field,
                visibility: Some(Visibility::Public),
                start_line: line_no,
                end_line: line_no,
                start_col: 0,
                end_col: 0,
                signature: Some(format!("h{level}")),
                doc_comment: None,
                scope_path: Some(file_name.clone()),
                parent_index: Some(host_index),
            });
        }
        // Inline link / image references. One pass per line picks them
        // up cheaply without tree-sitter.
        collect_link_refs(line, line_no, host_index, &mut refs);
        line_no += 1;
        ls = le + 1;
    }

    // Fenced-block anchor symbols — one per fence, regardless of whether
    // the info-string normalizes to a known language.
    for (idx, fence) in fenced::parse_fences(source).iter().enumerate() {
        let lang = info_string::normalize(&fence.info).unwrap_or("text");
        let anchor = format!("{lang}#{idx}");
        symbols.push(ExtractedSymbol {
            name: anchor.clone(),
            qualified_name: format!("{file_name}.{anchor}"),
            kind: SymbolKind::Class,
            visibility: Some(Visibility::Public),
            start_line: fence.body_line_offset,
            end_line: fence.body_line_offset
                + fence.body.matches('\n').count() as u32,
            start_col: 0,
            end_col: 0,
            signature: Some(fence.info.clone()),
            doc_comment: None,
            scope_path: Some(file_name.clone()),
            parent_index: Some(host_index),
        });
    }

    ExtractionResult {
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: false,
    }
}

fn parse_atx_heading(line: &[u8]) -> Option<(u32, String)> {
    let mut i = 0;
    while i < line.len() && i < 3 && line[i] == b' ' {
        i += 1;
    }
    let mut level = 0u32;
    while i < line.len() && line[i] == b'#' && level < 6 {
        level += 1;
        i += 1;
    }
    if level == 0 {
        return None;
    }
    // Must be followed by space or EOL.
    if i < line.len() && line[i] != b' ' && line[i] != b'\t' {
        return None;
    }
    while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
        i += 1;
    }
    let mut end = line.len();
    // Strip trailing `#`s (optional closing sequence) and whitespace.
    while end > i && (line[end - 1] == b' ' || line[end - 1] == b'\t' || line[end - 1] == b'\r') {
        end -= 1;
    }
    while end > i && line[end - 1] == b'#' {
        end -= 1;
    }
    while end > i && (line[end - 1] == b' ' || line[end - 1] == b'\t') {
        end -= 1;
    }
    let text = std::str::from_utf8(&line[i..end]).ok()?.to_string();
    if text.is_empty() {
        return None;
    }
    Some((level, text))
}

fn collect_link_refs(
    line: &[u8],
    line_no: u32,
    host_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let s = match std::str::from_utf8(line) {
        Ok(s) => s,
        Err(_) => return,
    };
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' || (chars[i] == '!' && chars.get(i + 1) == Some(&'[')) {
            let open = if chars[i] == '!' { i + 1 } else { i };
            if let Some(close) = find_match_bracket(&chars, open) {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(paren_close) = find_match_paren(&chars, close + 1) {
                        let target: String = chars[close + 2..paren_close].iter().collect();
                        if let Some(normalized) = normalize_link_target(&target) {
                            refs.push(ExtractedRef {
                                source_symbol_index: host_index,
                                target_name: normalized,
                                kind: EdgeKind::Imports,
                                line: line_no,
                                module: None,
                                chain: None,
                            });
                        }
                        i = paren_close + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
}

fn find_match_bracket(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, &c) in chars.iter().enumerate().skip(open) {
        if c == '[' {
            depth += 1;
        } else if c == ']' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

fn find_match_paren(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, &c) in chars.iter().enumerate().skip(open) {
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

fn normalize_link_target(target: &str) -> Option<String> {
    let target = target.split_whitespace().next()?;
    // Strip title (after whitespace inside parens is already gone).
    // Skip fragments (`#anchor`), mailto:, external URLs, and empty.
    if target.is_empty() || target.starts_with('#') {
        return None;
    }
    if target.contains("://") || target.starts_with("mailto:") {
        return None;
    }
    // Path normalization: strip leading ./, trailing anchors, and the
    // extension. A link `./architecture/overview.md#intro` becomes
    // `architecture/overview` — the resolver matches this against the
    // file stem of the target markdown file.
    let mut t = target;
    if let Some(stripped) = t.strip_prefix("./") {
        t = stripped;
    }
    if let Some(pos) = t.find('#') {
        t = &t[..pos];
    }
    if t.is_empty() {
        return None;
    }
    let path = std::path::Path::new(t);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(t);
    let parent = path
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or("");
    let normalized = if parent.is_empty() {
        stem.to_string()
    } else {
        format!("{}/{}", parent.replace('\\', "/"), stem)
    };
    Some(normalized)
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

fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_atx_headings() {
        let src = "# Top\n\n## Sub\n\n### Deeper ###\n";
        let r = extract(src, "README.md");
        let h: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Field)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(h, vec!["Top", "Sub", "Deeper"]);
    }

    #[test]
    fn emits_file_host_symbol() {
        let src = "plain\n";
        let r = extract(src, "docs/overview.md");
        assert_eq!(r.symbols[0].name, "overview");
        assert_eq!(r.symbols[0].kind, SymbolKind::Class);
    }

    #[test]
    fn emits_fence_anchors() {
        let src = "```ts\nlet x = 1;\n```\n\n```python\nprint('x')\n```\n";
        let r = extract(src, "README.md");
        let anchors: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class && s.scope_path.is_some())
            .map(|s| s.name.as_str())
            .collect();
        assert!(anchors.contains(&"typescript#0"));
        assert!(anchors.contains(&"python#1"));
    }

    #[test]
    fn unknown_info_string_still_anchored_as_text() {
        let src = "```mermaid\ngraph\n```\n";
        let r = extract(src, "README.md");
        assert!(r.symbols.iter().any(|s| s.name == "text#0"));
    }

    #[test]
    fn relative_link_becomes_imports_ref() {
        let src = "See [overview](./architecture/overview.md) for details.\n";
        let r = extract(src, "README.md");
        let ref_targets: Vec<&str> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
        assert_eq!(ref_targets, vec!["architecture/overview"]);
        assert_eq!(r.refs[0].kind, EdgeKind::Imports);
    }

    #[test]
    fn external_link_ignored() {
        let src = "[site](https://example.com/foo) [mail](mailto:a@b.c)\n";
        let r = extract(src, "README.md");
        assert!(r.refs.is_empty());
    }

    #[test]
    fn anchor_only_link_ignored() {
        let src = "See [intro](#intro).\n";
        let r = extract(src, "README.md");
        assert!(r.refs.is_empty());
    }

    #[test]
    fn image_link_becomes_ref() {
        let src = "![alt](./images/logo.png)\n";
        let r = extract(src, "README.md");
        assert_eq!(r.refs.len(), 1);
        assert_eq!(r.refs[0].target_name, "images/logo");
    }
}
