//! PHP host embedded-region detection.
//!
//! The tree-sitter-php grammar handles `<?php … ?>` mode-switching
//! internally, but it treats the HTML sections between PHP blocks as
//! inert text. This means inline `<script>` and `<style>` blocks in
//! a `.php` file's HTML regions never get sub-extracted unless the host
//! emits them as embedded regions.
//!
//! Strategy: scan the source linearly, tracking whether the byte cursor
//! sits inside a PHP open/close pair. When in HTML mode, scan for
//! `<script>...</script>` and `<style>...</style>` blocks and emit
//! `EmbeddedRegion`s. When in PHP mode, skip past the matching `?>`.
//!
//! PHP open tags recognised:
//!   * `<?php` — standard
//!   * `<?=`   — short echo
//!   * `<?`    — short tag (only if `short_open_tag=On` in php.ini, but
//!               we treat it as PHP universally — false positives are
//!               rare and the cost is just skipping until `?>` which is
//!               safe in every legitimate PHP file)
//!
//! Files with no PHP tag at all (rare for `.php`) are scanned as one
//! HTML region. Files that never close their PHP tag are scanned as one
//! PHP region with no embedded output.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let bytes = source.as_bytes();
    let mut regions = Vec::new();

    // Track HTML-mode runs and scan each for script/style blocks.
    let mut i = 0;
    let mut html_start: Option<usize> = Some(0);
    while i < bytes.len() {
        if let Some(open) = match_php_open(bytes, i) {
            // Flush any HTML region accumulated so far.
            if let Some(start) = html_start.take() {
                scan_html_section(source, bytes, start, i, &mut regions);
            }
            // Skip past matching `?>` (or to EOF).
            i = match find_php_close(bytes, open) {
                Some(close_end) => close_end,
                None => bytes.len(),
            };
            html_start = Some(i);
            continue;
        }
        i += 1;
    }
    if let Some(start) = html_start {
        scan_html_section(source, bytes, start, bytes.len(), &mut regions);
    }
    regions
}

/// At byte `i`, return the byte AFTER a recognised PHP open tag, or
/// `None`. Recognises `<?php` (whitespace required after), `<?=`, and
/// the bare short tag `<?`.
fn match_php_open(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i) != Some(&b'<') || bytes.get(i + 1) != Some(&b'?') {
        return None;
    }
    // `<?php` — must be followed by whitespace or end-of-file.
    if has_prefix(bytes, i + 2, b"php") {
        let after = i + 5;
        if after >= bytes.len() || bytes[after].is_ascii_whitespace() {
            return Some(after);
        }
    }
    // `<?=` short echo
    if bytes.get(i + 2) == Some(&b'=') {
        return Some(i + 3);
    }
    // Bare `<?` short tag — accept everything else that's not <?xml.
    if has_prefix(bytes, i + 2, b"xml") {
        return None;
    }
    Some(i + 2)
}

/// Find the byte AFTER the next `?>` close tag at or after `start`.
/// Returns `None` if the file ends with PHP still open.
fn find_php_close(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < bytes.len() {
        if bytes[i] == b'?' && bytes[i + 1] == b'>' {
            return Some(i + 2);
        }
        // Walk past PHP strings so a `?>` inside a string literal
        // doesn't terminate the block prematurely. PHP supports
        // single-quoted, double-quoted, heredoc and nowdoc strings;
        // single + double cover the vast majority of false positives.
        match bytes[i] {
            b'"' => i = skip_string(bytes, i, b'"'),
            b'\'' => i = skip_string(bytes, i, b'\''),
            b'/' if bytes.get(i + 1) == Some(&b'/') => i = skip_to_eol(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_block_comment(bytes, i),
            b'#' if bytes.get(i + 1) != Some(&b'[') => i = skip_to_eol(bytes, i),
            _ => i += 1,
        }
    }
    None
}

fn skip_string(bytes: &[u8], pos: usize, quote: u8) -> usize {
    let mut i = pos + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b if b == quote => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn skip_to_eol(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
    i
}

fn skip_block_comment(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos + 2;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            return i + 2;
        }
        i += 1;
    }
    bytes.len()
}

/// Scan `source[start..end]` (an HTML region between PHP tags) for
/// `<script>` / `<style>` blocks and append regions to `out`.
fn scan_html_section(
    source: &str,
    bytes: &[u8],
    start: usize,
    end: usize,
    out: &mut Vec<EmbeddedRegion>,
) {
    if start >= end { return; }
    let mut i = start;
    while i < end {
        if bytes[i] == b'<' {
            if has_prefix_ci(bytes, i + 1, b"script") {
                if let Some((body_start, body_end, after, lang)) =
                    match_html_block(bytes, i, b"script", end)
                {
                    if let Some(content) = source.get(body_start..body_end) {
                        if !content.is_empty() {
                            out.push(make_region(
                                source,
                                body_start,
                                content,
                                lang.unwrap_or("javascript"),
                                EmbeddedOrigin::ScriptBlock,
                            ));
                        }
                    }
                    i = after;
                    continue;
                }
            } else if has_prefix_ci(bytes, i + 1, b"style") {
                if let Some((body_start, body_end, after, lang)) =
                    match_html_block(bytes, i, b"style", end)
                {
                    if let Some(content) = source.get(body_start..body_end) {
                        if !content.is_empty() {
                            out.push(make_region(
                                source,
                                body_start,
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
}

/// Match `<{tag} ...>...body...</{tag}>` (case-insensitive) bounded at
/// `end`. Returns `(body_start, body_end, past_close_tag, language_id)`
/// where `language_id` reflects `lang=` / `type=` attributes for script
/// (typescript / javascript) or style (scss / css).
fn match_html_block(
    bytes: &[u8],
    tag_start: usize,
    tag: &[u8],
    end: usize,
) -> Option<(usize, usize, usize, Option<&'static str>)> {
    if !has_prefix_ci(bytes, tag_start + 1, tag) { return None; }
    let tag_end = (tag_start + 1 + tag.len()..end)
        .find(|&i| bytes[i] == b'>')?;
    if bytes.get(tag_end.saturating_sub(1)) == Some(&b'/') { return None; }
    let attrs = &bytes[tag_start..tag_end];
    let lang = if tag == b"script" {
        Some(script_language_from_attrs(attrs))
    } else if tag == b"style" {
        Some(style_language_from_attrs(attrs))
    } else {
        None
    };
    let body_start = tag_end + 1;
    let close_needle: Vec<u8> = {
        let mut v = Vec::with_capacity(tag.len() + 3);
        v.extend_from_slice(b"</");
        v.extend_from_slice(tag);
        v
    };
    let _ = close_needle; // only used for sizing intent — bounds enforced inline.
    let mut i = body_start;
    while i < end {
        if bytes[i] == b'<'
            && bytes.get(i + 1) == Some(&b'/')
            && has_prefix_ci(bytes, i + 2, tag)
        {
            let after_name = i + 2 + tag.len();
            if let Some(gt) = (after_name..end).find(|&j| bytes[j] == b'>') {
                return Some((body_start, i, gt + 1, lang));
            }
        }
        i += 1;
    }
    None
}

fn script_language_from_attrs(attr_bytes: &[u8]) -> &'static str {
    let s = std::str::from_utf8(attr_bytes).unwrap_or("");
    let lower = s.to_ascii_lowercase();
    if lower.contains("lang=\"ts\"")
        || lower.contains("lang='ts'")
        || lower.contains("lang=\"typescript\"")
        || lower.contains("lang='typescript'")
        || lower.contains("type=\"text/typescript\"")
        || lower.contains("type='text/typescript'")
    {
        "typescript"
    } else {
        "javascript"
    }
}

fn style_language_from_attrs(attr_bytes: &[u8]) -> &'static str {
    let s = std::str::from_utf8(attr_bytes).unwrap_or("");
    let lower = s.to_ascii_lowercase();
    if lower.contains("lang=\"scss\"") || lower.contains("lang='scss'") {
        "scss"
    } else {
        "css"
    }
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_php_no_html_emits_no_regions() {
        let src = "<?php\nfunction f() { return 1; }\n";
        assert!(detect_regions(src).is_empty());
    }

    #[test]
    fn html_only_with_script_emits_js_region() {
        let src = "<html><body><script>function go(){}</script></body></html>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "javascript");
        assert!(regions[0].text.contains("function go"));
    }

    #[test]
    fn php_with_inline_script_after_close_tag() {
        let src = "<?php $name = 'Alice'; ?>\n<script>console.log('hi');</script>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "javascript");
        assert!(regions[0].text.contains("console.log"));
    }

    #[test]
    fn script_inside_php_string_is_not_detected() {
        // `<script>` appears inside a PHP string — must NOT be picked up.
        let src = "<?php $x = '<script>nope</script>'; ?>";
        let regions = detect_regions(src);
        assert!(regions.is_empty(), "got {regions:?}");
    }

    #[test]
    fn alternating_php_and_html_blocks() {
        let src = "<?php echo 'a'; ?>\n<script>x=1;</script>\n<?php echo 'b'; ?>\n<style>p{}</style>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 2);
        assert!(regions.iter().any(|r| r.language_id == "javascript"));
        assert!(regions.iter().any(|r| r.language_id == "css"));
    }

    #[test]
    fn script_with_lang_ts_emits_typescript() {
        let src = "<script lang=\"ts\">const x: number = 1;</script>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "typescript");
    }

    #[test]
    fn style_with_lang_scss() {
        let src = "<style lang=\"scss\">.x { color: red; }</style>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "scss");
    }

    #[test]
    fn unclosed_php_emits_no_regions() {
        let src = "<?php $x = 1; // never closes\n<script>nope</script>";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn short_echo_tag_recognised() {
        let src = "<p><?= $name ?></p><script>x=1;</script>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "javascript");
    }

    #[test]
    fn xml_declaration_is_not_php() {
        // `<?xml ...?>` shouldn't be treated as a PHP block.
        let src = "<?xml version=\"1.0\"?><script>x=1;</script>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "javascript");
    }
}
