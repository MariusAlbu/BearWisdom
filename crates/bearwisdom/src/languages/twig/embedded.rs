//! Twig region detection.
//!
//! Emits `<script>` and `<style>` regions for sub-extraction. Twig
//! expressions (`{{ expr }}`) and directive bodies (`{% ... %}`) are
//! NOT routed through a sub-extractor — Twig has no expression grammar
//! in the workspace, and its filter / pipe syntax doesn't usefully map
//! onto PHP or Python without significant adapter work. The host
//! extractor in `extract.rs` already surfaces every queryable Twig
//! construct (block, macro, extends, include, use, import, from, embed)
//! as a real symbol or ref, so leaving expressions un-extracted doesn't
//! lose graph signal.
//!
//! Twig comments `{# ... #}` are skipped so a `<script>` mention inside
//! a comment doesn't trigger detection.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let bytes = source.as_bytes();
    let mut regions = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // `{# ... #}` Twig comment.
        if has_prefix(bytes, i, b"{#") {
            if let Some(end) = find_subseq(bytes, i + 2, b"#}") {
                i = end + 2;
                continue;
            }
            break;
        }
        if bytes[i] == b'<' {
            if has_prefix_ci(bytes, i + 1, b"script") {
                if let Some((bs, be, after, lang)) = match_html_block(bytes, i, b"script") {
                    if let Some(content) = source.get(bs..be) {
                        if !content.is_empty() {
                            regions.push(make_region(
                                source, bs, content,
                                lang.unwrap_or("javascript"),
                                EmbeddedOrigin::ScriptBlock,
                            ));
                        }
                    }
                    i = after;
                    continue;
                }
            }
            if has_prefix_ci(bytes, i + 1, b"style") {
                if let Some((bs, be, after, lang)) = match_html_block(bytes, i, b"style") {
                    if let Some(content) = source.get(bs..be) {
                        if !content.is_empty() {
                            regions.push(make_region(
                                source, bs, content,
                                lang.unwrap_or("css"),
                                EmbeddedOrigin::StyleBlock,
                            ));
                        }
                    }
                    i = after;
                    continue;
                }
            }
        }
        i += 1;
    }
    regions
}

fn make_region(
    source: &str, byte_start: usize, content: &str,
    language_id: &'static str, origin: EmbeddedOrigin,
) -> EmbeddedRegion {
    let (line, col) = line_col_at(source.as_bytes(), byte_start);
    EmbeddedRegion {
        language_id: language_id.to_string(),
        text: content.to_string(),
        line_offset: line,
        col_offset: col,
        origin,
        holes: Vec::new(),
        strip_scope_prefix: None,
    }
}

fn match_html_block(
    bytes: &[u8], tag_start: usize, tag: &[u8],
) -> Option<(usize, usize, usize, Option<&'static str>)> {
    if !has_prefix_ci(bytes, tag_start + 1, tag) { return None; }
    let tag_end = (tag_start + 1 + tag.len()..bytes.len())
        .find(|&i| bytes[i] == b'>')?;
    if bytes.get(tag_end.saturating_sub(1)) == Some(&b'/') { return None; }
    let attrs = &bytes[tag_start..tag_end];
    let lang = if tag == b"script" {
        Some(script_lang(attrs))
    } else if tag == b"style" {
        Some(style_lang(attrs))
    } else { None };
    let body_start = tag_end + 1;
    let mut i = body_start;
    while i < bytes.len() {
        if bytes[i] == b'<'
            && bytes.get(i + 1) == Some(&b'/')
            && has_prefix_ci(bytes, i + 2, tag)
        {
            let after_name = i + 2 + tag.len();
            if let Some(gt) = (after_name..bytes.len()).find(|&j| bytes[j] == b'>') {
                return Some((body_start, i, gt + 1, lang));
            }
        }
        i += 1;
    }
    None
}

fn script_lang(attrs: &[u8]) -> &'static str {
    let s = std::str::from_utf8(attrs).unwrap_or("").to_ascii_lowercase();
    if s.contains("lang=\"ts\"") || s.contains("lang='ts'")
        || s.contains("lang=\"typescript\"") || s.contains("lang='typescript'")
    { "typescript" } else { "javascript" }
}

fn style_lang(attrs: &[u8]) -> &'static str {
    let s = std::str::from_utf8(attrs).unwrap_or("").to_ascii_lowercase();
    if s.contains("lang=\"scss\"") || s.contains("lang='scss'") { "scss" } else { "css" }
}

fn has_prefix(bytes: &[u8], start: usize, needle: &[u8]) -> bool {
    if start + needle.len() > bytes.len() { return false; }
    &bytes[start..start + needle.len()] == needle
}

fn has_prefix_ci(bytes: &[u8], start: usize, needle: &[u8]) -> bool {
    if start + needle.len() > bytes.len() { return false; }
    bytes[start..start + needle.len()]
        .iter()
        .zip(needle.iter())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn find_subseq(bytes: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || start > bytes.len() { return None; }
    let end = bytes.len().saturating_sub(needle.len()) + 1;
    (start..end).find(|&i| bytes[i..].starts_with(needle))
}

fn line_col_at(bytes: &[u8], byte_pos: usize) -> (u32, u32) {
    let mut line: u32 = 0;
    let mut last_nl: usize = 0;
    for (i, b) in bytes.iter().enumerate().take(byte_pos) {
        if *b == b'\n' {
            line += 1;
            last_nl = i + 1;
        }
    }
    (line, (byte_pos - last_nl) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_block_emits_js_region() {
        let src = "<h1>x</h1>\n<script>function f() {}</script>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "javascript");
    }

    #[test]
    fn style_block_emits_css_region() {
        let src = "<style>p{color:red}</style>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "css");
    }

    #[test]
    fn comment_hides_script_mention() {
        let src = "{# <script>fake</script> #}\n<script>real();</script>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("real()"));
    }

    #[test]
    fn no_regions_for_pure_twig() {
        let src = "{% block content %}Hello {{ name }}{% endblock %}";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }
}
