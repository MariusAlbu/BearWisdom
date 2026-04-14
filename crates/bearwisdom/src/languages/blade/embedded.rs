//! Blade region detection.
//!
//! Blade is a Laravel templating language layered on top of PHP. It has
//! no native tree-sitter grammar, so this module hand-rolls a detector
//! that splits a `.blade.php` source into embedded regions for
//! sub-extraction:
//!
//!   * `{{ expr }}`, `{!! expr !!}` — PHP expressions. Wrapped as a
//!     `<?php $_ = (EXPR); ?>` snippet so the PHP grammar parses them
//!     and surfaces type / function refs through the normal extractor.
//!     Origin = `TemplateExpr`.
//!
//!   * `@php ... @endphp` — PHP statement blocks. Wrapped as
//!     `<?php BODY ?>`. Origin = `PhpBlock`.
//!
//!   * `<script>...</script>` — JavaScript / TypeScript via `lang="ts"`.
//!     Origin = `ScriptBlock`.
//!
//!   * `<style>...</style>` — CSS / SCSS via `lang="scss"`. Origin =
//!     `StyleBlock`.
//!
//! Blade comments `{{-- ... --}}` are stripped from the scan so their
//! contents don't trigger nested matches. `@verbatim ... @endverbatim`
//! pauses Blade interpolation but we don't honor it (cost > value: any
//! literal `{{ }}` inside @verbatim would get parsed as PHP, and the
//! grammar's error recovery handles malformed input gracefully).

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let bytes = source.as_bytes();
    let mut regions = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        // Blade comment — `{{-- ... --}}` — skip entirely.
        if has_prefix(bytes, i, b"{{--") {
            if let Some(end) = find_subseq(bytes, i + 4, b"--}}") {
                i = end + 4;
                continue;
            }
            break; // unterminated comment — bail.
        }

        // `{!! raw expr !!}`
        if has_prefix(bytes, i, b"{!!") {
            if let Some(end) = find_subseq(bytes, i + 3, b"!!}") {
                let body_start = i + 3;
                let body_end = end;
                if let Some(content) = source.get(body_start..body_end).map(str::trim) {
                    if !content.is_empty() {
                        regions.push(make_php_expr_region(source, body_start, content));
                    }
                }
                i = end + 3;
                continue;
            }
            break;
        }

        // `{{ expr }}`
        if has_prefix(bytes, i, b"{{") {
            if let Some(end) = find_subseq(bytes, i + 2, b"}}") {
                let body_start = i + 2;
                let body_end = end;
                if let Some(content) = source.get(body_start..body_end).map(str::trim) {
                    if !content.is_empty() {
                        regions.push(make_php_expr_region(source, body_start, content));
                    }
                }
                i = end + 2;
                continue;
            }
            break;
        }

        // `@php ... @endphp`
        if has_prefix(bytes, i, b"@php") {
            // Word-boundary check.
            let after = i + 4;
            let next = bytes.get(after).copied().unwrap_or(b' ');
            if !next.is_ascii_alphanumeric() && next != b'_' {
                if let Some(end_kw) = find_subseq(bytes, after, b"@endphp") {
                    let body_start = after;
                    let body_end = end_kw;
                    if let Some(content) = source.get(body_start..body_end) {
                        let trimmed = content.trim();
                        if !trimmed.is_empty() {
                            regions.push(make_php_block_region(source, body_start, content));
                        }
                    }
                    i = end_kw + b"@endphp".len();
                    continue;
                }
            }
        }

        // `<script>...</script>` / `<style>...</style>`
        if bytes[i] == b'<' {
            if has_prefix_ci(bytes, i + 1, b"script") {
                if let Some((bs, be, after, lang)) =
                    match_html_block(bytes, i, b"script")
                {
                    if let Some(content) = source.get(bs..be) {
                        if !content.is_empty() {
                            regions.push(make_region(
                                source,
                                bs,
                                content,
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
                if let Some((bs, be, after, lang)) =
                    match_html_block(bytes, i, b"style")
                {
                    if let Some(content) = source.get(bs..be) {
                        if !content.is_empty() {
                            regions.push(make_region(
                                source,
                                bs,
                                content,
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

fn make_php_expr_region(source: &str, byte_start: usize, expr: &str) -> EmbeddedRegion {
    let (line, _col) = line_col_at(source.as_bytes(), byte_start);
    // `<?php $__blade = (EXPR); ?>` — valid statement that surfaces type
    // / function refs in EXPR through the PHP extractor. The synthetic
    // `$__blade` variable is not a useful symbol; the PHP plugin has no
    // strip_scope_prefix concept (variable refs aren't qualified by scope
    // path), so it doesn't pollute the index.
    let wrapped = format!("<?php $__blade = ({expr}); ?>");
    EmbeddedRegion {
        language_id: "php".to_string(),
        text: wrapped,
        line_offset: line,
        col_offset: 0,
        origin: EmbeddedOrigin::TemplateExpr,
        holes: Vec::new(),
        strip_scope_prefix: None,
    }
}

fn make_php_block_region(source: &str, byte_start: usize, body: &str) -> EmbeddedRegion {
    let (line, _col) = line_col_at(source.as_bytes(), byte_start);
    let wrapped = format!("<?php\n{body}\n?>");
    EmbeddedRegion {
        language_id: "php".to_string(),
        text: wrapped,
        line_offset: line.saturating_sub(1),
        col_offset: 0,
        origin: EmbeddedOrigin::PhpBlock,
        holes: Vec::new(),
        strip_scope_prefix: None,
    }
}

fn make_region(
    source: &str,
    byte_start: usize,
    content: &str,
    language_id: &'static str,
    origin: EmbeddedOrigin,
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
    bytes: &[u8],
    tag_start: usize,
    tag: &[u8],
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
    } else {
        None
    };
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
    fn double_brace_emits_php_expr() {
        let src = "<p>Hi, {{ $user->name }}</p>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "php");
        assert_eq!(regions[0].origin, EmbeddedOrigin::TemplateExpr);
        assert!(regions[0].text.contains("$user->name"));
        assert!(regions[0].text.starts_with("<?php"));
    }

    #[test]
    fn triple_bang_emits_php_expr() {
        let src = "{!! $html !!}";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "php");
        assert!(regions[0].text.contains("$html"));
    }

    #[test]
    fn at_php_block_emits_php_block() {
        let src = "@php\n$count = User::count();\n@endphp";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].origin, EmbeddedOrigin::PhpBlock);
        assert!(regions[0].text.contains("User::count()"));
    }

    #[test]
    fn script_block_default_javascript() {
        let src = "<script>console.log('hi');</script>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "javascript");
    }

    #[test]
    fn style_block_default_css() {
        let src = "<style>p{color:red}</style>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "css");
    }

    #[test]
    fn blade_comment_is_skipped() {
        let src = "{{-- {{ $hidden }} --}}\n{{ $real }}";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("$real"));
    }

    #[test]
    fn multiple_constructs_coexist() {
        let src = "@php $x = 1; @endphp\n<p>{{ $x }}</p>\n<script>x=1;</script>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 3);
        assert_eq!(
            regions.iter().filter(|r| r.language_id == "php").count(),
            2
        );
        assert!(regions.iter().any(|r| r.language_id == "javascript"));
    }

    #[test]
    fn unterminated_double_brace_does_not_panic() {
        let _ = detect_regions("{{ never closes");
    }

    #[test]
    fn empty_brace_emits_no_region() {
        let src = "{{ }}\n{{   }}";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }
}
