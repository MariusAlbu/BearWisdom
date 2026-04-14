//! MDX host-level extraction.
//!
//! Shared baseline (headings, fence anchors, link refs, file-stem
//! symbol) delegates to `markdown::host_scan`. MDX layers JSX
//! component refs on top: every `<Capital…>` or `<Foo.Bar>` opening
//! tag OUTSIDE of fenced code blocks becomes a `Calls` ref against
//! the host file symbol. Resolution later matches those refs against
//! components imported at the top of the file (dispatched as a TS
//! `ScriptBlock` region by `embedded.rs`) and against project-wide
//! component symbols.

use super::super::markdown::{fenced, host_scan};
use crate::types::{EdgeKind, ExtractedRef, ExtractionResult};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let mut scan = host_scan::scan(source, file_path);
    collect_jsx_refs(source, scan.host_index, &mut scan.refs);
    ExtractionResult {
        symbols: scan.symbols,
        refs: scan.refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: false,
    }
}

fn collect_jsx_refs(source: &str, host_index: usize, refs: &mut Vec<ExtractedRef>) {
    let fence_ranges = fence_byte_ranges(source);
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if inside_any_range(i, &fence_ranges) {
            // Skip to the end of the current fence.
            if let Some(end) = fence_ranges.iter().find(|(s, e)| i >= *s && i < *e).map(|(_, e)| *e) {
                i = end;
                continue;
            }
        }
        if bytes[i] == b'<' {
            if let Some((name, consumed)) = scan_jsx_tag(&bytes[i..]) {
                let line = line_of_byte(bytes, i);
                refs.push(ExtractedRef {
                    source_symbol_index: host_index,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line,
                    module: None,
                    chain: None,
                });
                i += consumed;
                continue;
            }
        }
        i += 1;
    }
}

/// Collect (start, end) byte ranges covering each fenced code block's
/// body (so JSX scanning skips inside fences).
fn fence_byte_ranges(source: &str) -> Vec<(usize, usize)> {
    fenced::parse_fences(source)
        .into_iter()
        .map(|f| (f.body_byte_offset, f.body_byte_offset + f.body.len()))
        .collect()
}

fn inside_any_range(pos: usize, ranges: &[(usize, usize)]) -> bool {
    ranges.iter().any(|(s, e)| pos >= *s && pos < *e)
}

/// If `bytes` starts with `<Name[.Qualifier]*` followed by a valid JSX
/// terminator (`>`, `/`, whitespace, or `\n`), return the tag name and
/// the number of bytes consumed (up to and including the `<Name…`
/// portion). Returns `None` for HTML lowercase tags, end tags
/// (`</...>`), fragments (`<>`), and comments (`<!--`).
fn scan_jsx_tag(bytes: &[u8]) -> Option<(String, usize)> {
    if bytes.first() != Some(&b'<') {
        return None;
    }
    if bytes.len() < 2 {
        return None;
    }
    let b1 = bytes[1];
    // Skip end tags, fragments, comments, doctypes.
    if b1 == b'/' || b1 == b'>' || b1 == b'!' || b1 == b'?' {
        return None;
    }
    // Must start with uppercase ASCII letter (or lowercase-then-dot, to
    // accept `<motion.div>` patterns). We accept lowercase starts only
    // when followed by a dot — that's the MDX convention for
    // namespaced imports like `framer-motion`'s `motion.`.
    if !b1.is_ascii_alphabetic() {
        return None;
    }
    let mut i = 1usize;
    let first_segment_start = i;
    while i < bytes.len() && is_jsx_ident_byte(bytes[i]) {
        i += 1;
    }
    let first_segment = std::str::from_utf8(&bytes[first_segment_start..i]).ok()?.to_string();
    let first_is_upper = first_segment
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_uppercase());
    let mut dotted_tail = String::new();
    let mut is_dotted = false;
    while i < bytes.len() && bytes[i] == b'.' {
        is_dotted = true;
        i += 1;
        let seg_start = i;
        while i < bytes.len() && is_jsx_ident_byte(bytes[i]) {
            i += 1;
        }
        if i == seg_start {
            return None;
        }
        let seg = std::str::from_utf8(&bytes[seg_start..i]).ok()?;
        dotted_tail.push('.');
        dotted_tail.push_str(seg);
    }
    // Must be terminated by whitespace, `/`, `>` — NOT by a further
    // identifier byte (we already consumed all of them).
    let term = bytes.get(i).copied().unwrap_or(b' ');
    let is_terminator = term == b' '
        || term == b'\t'
        || term == b'\n'
        || term == b'\r'
        || term == b'/'
        || term == b'>'
        || term == b'{';
    if !is_terminator {
        return None;
    }
    // Reject lowercase-starting tags unless they're dotted (e.g.
    // `motion.div`).
    if !first_is_upper && !is_dotted {
        return None;
    }
    let name = format!("{first_segment}{dotted_tail}");
    Some((name, i))
}

fn is_jsx_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn line_of_byte(bytes: &[u8], pos: usize) -> u32 {
    let mut line: u32 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if i >= pos {
            break;
        }
        if b == b'\n' {
            line += 1;
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SymbolKind;

    #[test]
    fn headings_extracted_like_markdown() {
        let src = "# Top\n\n## Sub\n";
        let r = extract(src, "page.mdx");
        let h: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Field)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(h, vec!["Top", "Sub"]);
    }

    #[test]
    fn file_stem_host_symbol_emitted() {
        let src = "plain\n";
        let r = extract(src, "content/post.mdx");
        assert_eq!(r.symbols[0].name, "post");
    }

    #[test]
    fn capitalized_jsx_becomes_calls_ref() {
        let src = "Hello.\n\n<Button variant=\"primary\">Click</Button>\n";
        let r = extract(src, "page.mdx");
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert_eq!(calls, vec!["Button"]);
    }

    #[test]
    fn self_closing_jsx_becomes_calls_ref() {
        let src = "<Hero title=\"Hi\" />\n";
        let r = extract(src, "page.mdx");
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert_eq!(calls, vec!["Hero"]);
    }

    #[test]
    fn dotted_jsx_becomes_calls_ref() {
        let src = "<Tabs.Root>\n<Tabs.Item />\n</Tabs.Root>\n";
        let r = extract(src, "page.mdx");
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        // `</Tabs.Root>` is an end tag and skipped.
        assert!(calls.contains(&"Tabs.Root"));
        assert!(calls.contains(&"Tabs.Item"));
    }

    #[test]
    fn lowercase_html_tag_is_not_a_ref() {
        let src = "A paragraph with <div>inner</div>.\n";
        let r = extract(src, "page.mdx");
        let calls_count = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .count();
        assert_eq!(calls_count, 0);
    }

    #[test]
    fn lowercase_dotted_accepted_motion_style() {
        // `framer-motion` usage: `<motion.div>` is a component ref.
        let src = "<motion.div animate={{ x: 1 }} />\n";
        let r = extract(src, "page.mdx");
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert_eq!(calls, vec!["motion.div"]);
    }

    #[test]
    fn fragment_tag_ignored() {
        let src = "<>\n<div />\n</>\n";
        let r = extract(src, "page.mdx");
        let calls_count = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .count();
        assert_eq!(calls_count, 0);
    }

    #[test]
    fn jsx_inside_fence_not_extracted() {
        let src = "```tsx\n<Button />\n```\n\n<Outside />\n";
        let r = extract(src, "page.mdx");
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        // `<Button />` is inside the tsx fence and must NOT emit a ref
        // from MDX's own scanner — the TS sub-extractor handles it.
        assert_eq!(calls, vec!["Outside"]);
    }

    #[test]
    fn relative_link_still_becomes_imports_ref() {
        let src = "See [details](./info.md).\n";
        let r = extract(src, "page.mdx");
        assert!(
            r.refs
                .iter()
                .any(|r| r.kind == EdgeKind::Imports && r.target_name == "info")
        );
    }

    #[test]
    fn end_tag_is_not_emitted_as_separate_ref() {
        let src = "<Card>body</Card>\n";
        let r = extract(src, "page.mdx");
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert_eq!(calls, vec!["Card"]);
    }
}
