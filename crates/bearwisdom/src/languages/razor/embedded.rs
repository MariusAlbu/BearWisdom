//! Razor region detection.
//!
//! Razor (`.cshtml` / `.razor`) has no native tree-sitter grammar that
//! parses the full mixed-mode syntax. This module hand-rolls a detector
//! that splits a Razor source into embedded regions for sub-extraction:
//!
//!   * `@{ ... }`             — C# statement block
//!   * `@code { ... }`        — Blazor C# method/field block
//!   * `@functions { ... }`   — MVC C# method/field block (legacy name)
//!   * `@(expr)`              — C# inline expression
//!   * `<script>...</script>` — JavaScript or TypeScript (lang="ts")
//!
//! Explicitly NOT handled in the MVP:
//!
//!   * `@model Foo` / `@inject` / `@using` / `@page` / `@layout` /
//!     `@inherits` / `@implements` / `@addTagHelper` / `@namespace`
//!     directives — these take a rest-of-line payload (often a bare type
//!     name, not a valid C# compilation unit). Emitting them as C# would
//!     produce error-only trees with no useful refs. Skipping them
//!     preserves the extraction signal; future work can emit them as
//!     host-level refs directly.
//!   * `@if (...) { ... }`, `@foreach`, `@while`, `@switch`, `@for` —
//!     control-flow constructs. Brace matching across the condition +
//!     body is more involved than MVP warrants; deferred.
//!   * `@identifier.chain` implicit expressions — delimiter detection
//!     is ambiguous with surrounding HTML.
//!
//! Razor comments `@* ... *@` are skipped entirely. `@@` escape sequences
//! (literal `@` in HTML) are passed through (they don't open a region).

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

/// Detect every embedded region in a Razor source file and return them
/// in emission order (the order is not load-bearing — the indexer runs
/// each region through its sub-language independently).
pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let bytes = source.as_bytes();
    let mut regions = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'@' {
            // `@* ... *@` Razor comment — skip past it.
            if has_prefix(bytes, i + 1, b"*") {
                if let Some(end) = find_subseq(bytes, i + 2, b"*@") {
                    i = end + 2;
                    continue;
                }
                // Unterminated — bail out of the whole scan rather than
                // loop forever.
                break;
            }
            // `@@` — escaped literal, skip both chars.
            if bytes.get(i + 1) == Some(&b'@') {
                i += 2;
                continue;
            }
            // `@{ ... }` code block.
            if let Some((content, body_start, end)) = match_at_brace(bytes, i + 1) {
                if let Some(region) =
                    make_csharp_region(source, body_start, content, EmbeddedOrigin::RazorCode)
                {
                    regions.push(region);
                }
                i = end;
                continue;
            }
            // `@code { ... }` / `@functions { ... }`.
            for keyword in &[b"code".as_slice(), b"functions".as_slice()] {
                if has_prefix(bytes, i + 1, keyword) {
                    let after_kw = i + 1 + keyword.len();
                    let after_ws = skip_ascii_ws(bytes, after_kw);
                    if bytes.get(after_ws) == Some(&b'{') {
                        if let Some((content, body_start, end)) =
                            match_brace_block(bytes, after_ws)
                        {
                            if let Some(region) = make_csharp_region(
                                source,
                                body_start,
                                content,
                                EmbeddedOrigin::RazorCode,
                            ) {
                                regions.push(region);
                            }
                            i = end;
                            break;
                        }
                    }
                }
            }
            if i != 0 && bytes.get(i) != Some(&b'@') {
                // `i` was advanced inside the keyword loop above.
                continue;
            }
            // `@(expr)` — inline expression.
            if bytes.get(i + 1) == Some(&b'(') {
                if let Some((content, body_start, end)) = match_paren_block(bytes, i + 1) {
                    if let Some(region) =
                        make_csharp_region(source, body_start, content, EmbeddedOrigin::RazorCode)
                    {
                        regions.push(region);
                    }
                    i = end;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        if b == b'<' && has_prefix(bytes, i + 1, b"script") {
            if let Some((body_start, body_end, end, lang)) = match_script_block(bytes, i) {
                if body_end > body_start {
                    if let Some(content) = source.get(body_start..body_end) {
                        if let Some(region) = make_region(
                            source,
                            body_start,
                            content,
                            lang,
                            EmbeddedOrigin::ScriptBlock,
                        ) {
                            regions.push(region);
                        }
                    }
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    regions
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn has_prefix(bytes: &[u8], start: usize, needle: &[u8]) -> bool {
    if start >= bytes.len() { return false; }
    bytes[start..].starts_with(needle)
}

fn find_subseq(bytes: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || start > bytes.len() { return None; }
    let end = bytes.len().saturating_sub(needle.len()) + 1;
    (start..end).find(|&i| bytes[i..].starts_with(needle))
}

fn skip_ascii_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
        i += 1;
    }
    i
}

/// Match `@` + immediately-following `{` … `}` pair. Returns
/// `(inner_text, body_start_byte, past_closing_brace_byte)` on success.
/// `at_pos` points at the char AFTER the `@`.
fn match_at_brace(bytes: &[u8], at_pos: usize) -> Option<(&str, usize, usize)> {
    if bytes.get(at_pos) != Some(&b'{') { return None; }
    let (content, body_start, end) = match_brace_block(bytes, at_pos)?;
    Some((content, body_start, end))
}

/// Match `{` … `}` starting at `open_pos`. Tracks depth, skips over
/// strings (`"..."` with `\"` escapes, `@"..."` verbatim, `$"..."`
/// interpolated — simplified), character literals (`'x'`, `'\\''`), and
/// single-line/block comments. Returns `(inner_text, body_start_byte,
/// past_closing_brace_byte)`.
fn match_brace_block(bytes: &[u8], open_pos: usize) -> Option<(&str, usize, usize)> {
    if bytes.get(open_pos) != Some(&b'{') { return None; }
    let body_start = open_pos + 1;
    let mut depth: i32 = 1;
    let mut i = body_start;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => { depth += 1; i += 1; }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let end = i + 1;
                    let content = std::str::from_utf8(&bytes[body_start..i]).ok()?;
                    return Some((content, body_start, end));
                }
                i += 1;
            }
            b'"' => { i = skip_csharp_string(bytes, i); }
            b'\'' => { i = skip_char_literal(bytes, i); }
            b'/' if bytes.get(i + 1) == Some(&b'/') => { i = skip_line_comment(bytes, i); }
            b'/' if bytes.get(i + 1) == Some(&b'*') => { i = skip_block_comment(bytes, i); }
            _ => i += 1,
        }
    }
    None
}

/// Match `(` … `)` pair starting at `open_pos`. Same string/comment
/// skipping as `match_brace_block`.
fn match_paren_block(bytes: &[u8], open_pos: usize) -> Option<(&str, usize, usize)> {
    if bytes.get(open_pos) != Some(&b'(') { return None; }
    let body_start = open_pos + 1;
    let mut depth: i32 = 1;
    let mut i = body_start;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => { depth += 1; i += 1; }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    let end = i + 1;
                    let content = std::str::from_utf8(&bytes[body_start..i]).ok()?;
                    return Some((content, body_start, end));
                }
                i += 1;
            }
            b'"' => { i = skip_csharp_string(bytes, i); }
            b'\'' => { i = skip_char_literal(bytes, i); }
            b'/' if bytes.get(i + 1) == Some(&b'/') => { i = skip_line_comment(bytes, i); }
            b'/' if bytes.get(i + 1) == Some(&b'*') => { i = skip_block_comment(bytes, i); }
            _ => i += 1,
        }
    }
    None
}

/// Skip a C# string literal starting at `pos` (where `bytes[pos] == '"'`).
/// Handles backslash escapes; `@"..."` verbatim strings and `$"..."`
/// interpolation are NOT specially handled in the MVP (their scan-stop
/// still lands on the terminating `"`, which is sufficient for our
/// shallow brace-match pass to not bleed into HTML).
fn skip_csharp_string(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn skip_char_literal(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\'' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn skip_line_comment(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos + 2;
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

/// Match a `<script>...</script>` block (case-insensitive). Returns
/// `(body_start, body_end, past_closing_tag, language_id)` where
/// `language_id` is "typescript" (via `type="text/typescript"` or
/// `lang="ts"`) or "javascript" (default).
fn match_script_block(
    bytes: &[u8],
    tag_start: usize,
) -> Option<(usize, usize, usize, &'static str)> {
    // Accept <script or <SCRIPT etc.
    if !case_insensitive_prefix(bytes, tag_start, b"<script") {
        return None;
    }
    let tag_end = find_byte(bytes, tag_start, b'>')?;
    // Bail on self-closing <script ... /> — no body.
    if bytes.get(tag_end.saturating_sub(1)) == Some(&b'/') {
        return None;
    }
    let attr_bytes = &bytes[tag_start..tag_end];
    let language = script_language_from_attrs(attr_bytes);
    let body_start = tag_end + 1;
    // Find </script> (case-insensitive).
    let end = find_close_script(bytes, body_start)?;
    Some((body_start, end.0, end.1, language))
}

/// Find `</script>` (case-insensitive) starting at `pos`. Returns
/// `(body_end_byte, past_close_tag_byte)`.
fn find_close_script(bytes: &[u8], pos: usize) -> Option<(usize, usize)> {
    let mut i = pos;
    while i + 8 < bytes.len() {
        if bytes[i] == b'<'
            && bytes.get(i + 1) == Some(&b'/')
            && case_insensitive_prefix(bytes, i + 2, b"script")
        {
            let after_name = i + 8;
            if let Some(gt) = find_byte(bytes, after_name, b'>') {
                return Some((i, gt + 1));
            }
        }
        i += 1;
    }
    None
}

fn find_byte(bytes: &[u8], start: usize, needle: u8) -> Option<usize> {
    (start..bytes.len()).find(|&i| bytes[i] == needle)
}

fn case_insensitive_prefix(bytes: &[u8], start: usize, needle: &[u8]) -> bool {
    if start + needle.len() > bytes.len() { return false; }
    bytes[start..start + needle.len()]
        .iter()
        .zip(needle.iter())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
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

/// Build an EmbeddedRegion from a byte range inside `source`. Computes
/// the line_offset and col_offset of the first byte by walking `source`
/// up to `byte_start`.
/// Wrap a Razor C# region body in a synthetic `class __RazorBody { … }`
/// compilation unit so tree-sitter-csharp sees a valid top-level
/// declaration. Bare method / property / field declarations at the root
/// of a compilation unit produce an empty parse under the default C#
/// grammar, which would swallow every `@{ ... }` / `@code { ... }` body.
///
/// Cost of the wrapper:
///   * Extracted `qualified_name`s carry a `__RazorBody.` prefix
///     (e.g. `__RazorBody.Increment`). Still resolvable — the prefix is
///     stable and filterable at query time.
///   * `col_offset` is forced to 0 because the wrapper pushes content
///     onto wrapped-line 1, not line 0 where col_offset applies. Line
///     numbers stay correct via `line_offset = content_line - 1`.
fn make_csharp_region(
    source: &str,
    byte_start: usize,
    content: &str,
    origin: EmbeddedOrigin,
) -> Option<EmbeddedRegion> {
    if content.is_empty() { return None; }
    let (line, _col) = line_col_at(source.as_bytes(), byte_start);
    let wrapped = format!("class __RazorBody {{\n{}\n}}", content);
    Some(EmbeddedRegion {
        language_id: "csharp".to_string(),
        text: wrapped,
        line_offset: line.saturating_sub(1),
        col_offset: 0,
        origin,
        holes: Vec::new(),
    })
}

fn make_region(
    source: &str,
    byte_start: usize,
    content: &str,
    language_id: &'static str,
    origin: EmbeddedOrigin,
) -> Option<EmbeddedRegion> {
    if content.is_empty() { return None; }
    let (line, col) = line_col_at(source.as_bytes(), byte_start);
    Some(EmbeddedRegion {
        language_id: language_id.to_string(),
        text: content.to_string(),
        line_offset: line,
        col_offset: col,
        origin,
        holes: Vec::new(),
    })
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
    let col = (byte_pos - last_nl) as u32;
    (line, col)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_brace_code_block() {
        let src = "<h1>Hello</h1>\n@{ var x = 1; var y = 2; }\n<p>done</p>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "csharp");
        assert_eq!(regions[0].origin, EmbeddedOrigin::RazorCode);
        // Wrapped body — original content is wrapped in `class __RazorBody {…}`.
        assert!(regions[0].text.contains("var x = 1; var y = 2;"));
        assert!(regions[0].text.contains("class __RazorBody"));
        // line_offset = content_start_line - 1; content is on line 1, so 0.
        assert_eq!(regions[0].line_offset, 0);
    }

    #[test]
    fn at_brace_with_nested_braces() {
        let src = "@{ var o = new { A = 1, B = new { C = 2 } }; }";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("new { A = 1"));
        assert!(regions[0].text.contains("C = 2"));
    }

    #[test]
    fn code_and_functions_blocks() {
        let src = "@code { int Count { get; set; } }\n@functions { void Do() {} }";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 2);
        assert!(regions[0].text.contains("Count"));
        assert!(regions[1].text.contains("Do"));
        // Both must be wrapped.
        assert!(regions[0].text.contains("class __RazorBody"));
        assert!(regions[1].text.contains("class __RazorBody"));
    }

    #[test]
    fn at_paren_inline_expression() {
        let src = "<p>Name: @(user.Name)</p>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "csharp");
        assert!(regions[0].text.contains("user.Name"));
    }

    #[test]
    fn script_block_default_is_javascript() {
        let src = "<h1>hi</h1>\n<script>function f() { return 42; }</script>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "javascript");
        assert_eq!(regions[0].origin, EmbeddedOrigin::ScriptBlock);
        assert_eq!(regions[0].text, "function f() { return 42; }");
    }

    #[test]
    fn script_block_with_lang_ts_is_typescript() {
        let src = "<script lang=\"ts\">const x: number = 1;</script>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "typescript");
    }

    #[test]
    fn razor_comment_is_skipped() {
        let src = "@* this is @{ a comment } *@\n@{ var x = 1; }";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1, "only the real @{{ }} should be detected");
        assert!(regions[0].text.contains("var x = 1"));
    }

    #[test]
    fn at_at_escape_is_ignored() {
        let src = "Email: user@@example.com\n@{ var y = 2; }";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("var y = 2"));
    }

    #[test]
    fn strings_inside_block_do_not_terminate_early() {
        // The `}` inside the string literal must not close the block.
        let src = "@{ var s = \"}a{\"; var t = 1; }";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("var t = 1"));
    }

    #[test]
    fn multiple_blocks_coexist() {
        let src = "@{ var a = 1; }\n<h1>x</h1>\n@(a.ToString())\n<script>alert('hi');</script>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 3);
        assert!(regions.iter().any(|r| r.language_id == "javascript"));
        assert_eq!(
            regions.iter().filter(|r| r.language_id == "csharp").count(),
            2
        );
    }

    #[test]
    fn line_offset_points_to_content_start() {
        // With the csharp wrapper, line_offset = content_line - 1.
        // Content is on line 2 (third line) → line_offset == 1.
        let src = "line0\nline1\n@{ body }";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].line_offset, 1);
    }

    #[test]
    fn no_regions_in_plain_html() {
        let src = "<html><body><h1>Hello</h1></body></html>";
        assert!(detect_regions(src).is_empty());
    }

    #[test]
    fn unterminated_block_does_not_loop_forever() {
        let src = "@{ var x = 1; // missing close";
        let _ = detect_regions(src);
    }
}
