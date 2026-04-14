//! Razor region detection.
//!
//! Razor (`.cshtml` / `.razor`) has no native tree-sitter grammar that
//! parses the full mixed-mode syntax. This module hand-rolls a detector
//! that splits a Razor source into embedded regions for sub-extraction:
//!
//!   * `@{ ... }`, `@code { ... }`, `@functions { ... }` — C# statement
//!     / member blocks. Body is wrapped in `class __RazorBody { … }` so
//!     tree-sitter-csharp accepts bare declarations; the synthetic class
//!     name is stripped post-dispatch via `strip_scope_prefix`.
//!
//!   * `@(expr)` — C# inline expression, wrapped the same way.
//!
//!   * `@model Foo`, `@inject Foo svc`, `@inherits Base<TModel>`,
//!     `@implements IFoo`, `@using X.Y.Z`, `@namespace X.Y` — Razor
//!     directives. Each rewrites its rest-of-line payload into a tiny
//!     valid C# compilation unit so the payload's type refs surface
//!     through the normal C# extractor.
//!
//!   * `@if (cond) { body }`, `@foreach`, `@while`, `@switch`, `@for`,
//!     `@using (disposable) { body }` — Razor control-flow constructs.
//!     Parsed as `keyword(cond) { body }` and wrapped in a synthetic
//!     method so the C# extractor sees a valid statement.
//!
//!   * `<script>...</script>` — JavaScript (default) or TypeScript
//!     (when `lang="ts"` or `type="text/typescript"`).
//!
//! Razor comments `@* ... *@` are skipped entirely. `@@` escapes pass
//! through without triggering region detection. Implicit expressions
//! (`@identifier.chain`) are not detected — their delimiters are
//! ambiguous against surrounding HTML.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

/// Detect every embedded region in a Razor source file and return them
/// in emission order. Order is not load-bearing — the indexer runs each
/// region through its sub-language independently.
pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let bytes = source.as_bytes();
    let mut regions = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];

        if b == b'@' {
            // `@*  ... *@` Razor comment — skip past it.
            if has_prefix(bytes, i + 1, b"*") {
                if let Some(end) = find_subseq(bytes, i + 2, b"*@") {
                    i = end + 2;
                    continue;
                }
                break; // unterminated comment — bail.
            }
            // `@@` escape — two chars, not a region start.
            if bytes.get(i + 1) == Some(&b'@') {
                i += 2;
                continue;
            }

            // Try each Razor construct in priority order. The first match
            // consumes the slice and advances `i`.
            if let Some((region, end)) = try_control_flow(source, bytes, i) {
                if let Some(r) = region { regions.push(r); }
                i = end;
                continue;
            }
            if let Some((region, end)) = try_directive(source, bytes, i) {
                if let Some(r) = region { regions.push(r); }
                i = end;
                continue;
            }
            if let Some((region, end)) = try_code_or_functions(source, bytes, i) {
                if let Some(r) = region { regions.push(r); }
                i = end;
                continue;
            }
            if let Some((region, end)) = try_at_brace(source, bytes, i) {
                if let Some(r) = region { regions.push(r); }
                i = end;
                continue;
            }
            if let Some((region, end)) = try_at_paren(source, bytes, i) {
                if let Some(r) = region { regions.push(r); }
                i = end;
                continue;
            }

            // Unrecognized `@` — treat as literal and move on.
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
// Constructs — each `try_*` returns `Some((region, consumed_end_byte))` when
// it matches at position `at`. The region itself is optional — directives
// with empty payloads consume the bytes but emit no region.
// ---------------------------------------------------------------------------

/// `@if|@foreach|@while|@switch|@for (cond) { body }` or
/// `@using (disposable) { body }`. Matched BEFORE `@using` namespace
/// directive so the paren-first form wins.
fn try_control_flow(
    source: &str,
    bytes: &[u8],
    at: usize,
) -> Option<(Option<EmbeddedRegion>, usize)> {
    static KEYWORDS: &[&[u8]] = &[b"if", b"foreach", b"while", b"switch", b"for"];

    let kw_start = at + 1;
    let (keyword, after_kw) = match_keyword(bytes, kw_start, KEYWORDS)
        .or_else(|| match_using_with_paren(bytes, kw_start))?;

    let paren_pos = skip_ascii_ws(bytes, after_kw);
    if bytes.get(paren_pos) != Some(&b'(') { return None; }
    let (cond, _cond_body_start, after_cond) = match_paren_block(bytes, paren_pos)?;

    let brace_pos = skip_ascii_ws(bytes, after_cond);
    if bytes.get(brace_pos) != Some(&b'{') { return None; }
    let (body, _body_start, end) = match_brace_block(bytes, brace_pos)?;

    // Rebuild the full construct text: `keyword (cond) { body }`.
    let construct = format!("{keyword} ({cond}) {{{body}}}");
    let (line, _col) = line_col_at(bytes, at);

    // Wrap as a method body so the C# extractor parses it as a statement.
    // Using `class __RazorBody { void __M() { … } }` means type refs in
    // the condition and body both surface through the normal extractor.
    let wrapped = format!("class __RazorBody {{\n void __M() {{\n{construct}\n}}\n}}\n");
    Some((
        Some(EmbeddedRegion {
            language_id: "csharp".to_string(),
            text: wrapped,
            // Wrapper adds 2 lines before the construct → line - 2.
            line_offset: line.saturating_sub(2),
            col_offset: 0,
            origin: EmbeddedOrigin::RazorCode,
            holes: Vec::new(),
            strip_scope_prefix: Some("__RazorBody".to_string()),
        }),
        end,
    ))
}

/// `@using (` → using-statement. Returns the keyword "using" and the
/// byte position AFTER `using`.
fn match_using_with_paren(bytes: &[u8], kw_start: usize) -> Option<(&'static str, usize)> {
    if !has_prefix(bytes, kw_start, b"using") { return None; }
    let after = kw_start + 5;
    let peek = skip_ascii_ws(bytes, after);
    if bytes.get(peek) == Some(&b'(') { Some(("using", after)) } else { None }
}

/// Try a list of keywords; return the one that matches plus the byte
/// position immediately after it. Checks word boundary to avoid matching
/// `@ifable`.
fn match_keyword<'a>(bytes: &[u8], at: usize, keywords: &'a [&'a [u8]]) -> Option<(&'a str, usize)> {
    for kw in keywords {
        if has_prefix(bytes, at, kw) {
            let end = at + kw.len();
            // Word boundary — next char must be non-ident.
            let next = bytes.get(end).copied().unwrap_or(b' ');
            if !is_ident_continue(next) {
                // Safe: keywords are ASCII.
                let kw_str = std::str::from_utf8(kw).ok()?;
                return Some((kw_str, end));
            }
        }
    }
    None
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Razor directives — rest-of-line payload, terminated by `\n` (or `\r\n`)
/// or end of file. Each rewrites the payload into a mini C# compilation
/// unit appropriate to its semantics:
///
///   * `@model Foo`              → `class __RazorBody { Foo __razor_model; }`
///   * `@inject Foo svc`         → `class __RazorBody { Foo svc; }`
///   * `@inherits Base<TModel>`  → `class __RazorBody : Base<TModel> {}`
///   * `@implements IFoo, IBar`  → `class __RazorBody : IFoo, IBar {}`
///   * `@using X.Y.Z`            → `using X.Y.Z;\nclass __RazorBody {}`
///   * `@namespace X.Y`          → `namespace X.Y { class __RazorBody {} }`
///
/// Directives without a payload (empty rest-of-line) consume the bytes
/// but emit no region.
fn try_directive(
    source: &str,
    bytes: &[u8],
    at: usize,
) -> Option<(Option<EmbeddedRegion>, usize)> {
    // Keywords ordered so longer prefixes win (e.g. `implements` before
    // a hypothetical `imp`). `using` comes AFTER the control-flow check
    // in the caller so `@using (x) { }` doesn't land here.
    static DIRECTIVES: &[&[u8]] = &[
        b"model", b"inject", b"inherits", b"implements", b"using", b"namespace",
    ];
    let kw_start = at + 1;
    let (keyword, after_kw) = match_keyword(bytes, kw_start, DIRECTIVES)?;

    // Payload = rest of line, trimmed, without trailing semicolon.
    let line_end = find_line_end(bytes, after_kw);
    let raw = std::str::from_utf8(&bytes[after_kw..line_end]).ok()?.trim();
    let payload = raw.trim_end_matches(';').trim();
    if payload.is_empty() {
        return Some((None, line_end));
    }

    let (line, _col) = line_col_at(bytes, at);
    let wrapped = wrap_directive(keyword, payload);
    Some((
        Some(EmbeddedRegion {
            language_id: "csharp".to_string(),
            text: wrapped,
            // Directive payloads are on wrapper line 0 or 1 depending on
            // the wrapper shape; set line_offset = directive line so
            // navigation jumps to the right Razor source line even if
            // sub-column positions drift.
            line_offset: line,
            col_offset: 0,
            origin: EmbeddedOrigin::RazorCode,
            holes: Vec::new(),
            strip_scope_prefix: Some("__RazorBody".to_string()),
        }),
        line_end,
    ))
}

fn wrap_directive(keyword: &str, payload: &str) -> String {
    match keyword {
        "model" => format!("class __RazorBody {{ {payload} __razor_model; }}"),
        "inject" => format!("class __RazorBody {{ {payload}; }}"),
        "inherits" | "implements" => {
            format!("class __RazorBody : {payload} {{}}")
        }
        "using" => format!("using {payload};\nclass __RazorBody {{}}"),
        "namespace" => format!(
            "namespace {payload} {{ class __RazorBody {{}} }}"
        ),
        _ => format!("class __RazorBody {{ {payload}; }}"),
    }
}

/// `@code { ... }` or `@functions { ... }`.
fn try_code_or_functions(
    source: &str,
    bytes: &[u8],
    at: usize,
) -> Option<(Option<EmbeddedRegion>, usize)> {
    static KEYWORDS: &[&[u8]] = &[b"code", b"functions"];
    let kw_start = at + 1;
    let (_, after_kw) = match_keyword(bytes, kw_start, KEYWORDS)?;
    let brace_pos = skip_ascii_ws(bytes, after_kw);
    if bytes.get(brace_pos) != Some(&b'{') { return None; }
    let (content, body_start, end) = match_brace_block(bytes, brace_pos)?;
    let region = make_csharp_region(source, body_start, content, EmbeddedOrigin::RazorCode);
    Some((region, end))
}

/// `@{ ... }`.
fn try_at_brace(
    source: &str,
    bytes: &[u8],
    at: usize,
) -> Option<(Option<EmbeddedRegion>, usize)> {
    if bytes.get(at + 1) != Some(&b'{') { return None; }
    let (content, body_start, end) = match_brace_block(bytes, at + 1)?;
    let region = make_csharp_region(source, body_start, content, EmbeddedOrigin::RazorCode);
    Some((region, end))
}

/// `@(expr)`.
fn try_at_paren(
    source: &str,
    bytes: &[u8],
    at: usize,
) -> Option<(Option<EmbeddedRegion>, usize)> {
    if bytes.get(at + 1) != Some(&b'(') { return None; }
    let (content, body_start, end) = match_paren_block(bytes, at + 1)?;
    let region = make_csharp_region(source, body_start, content, EmbeddedOrigin::RazorCode);
    Some((region, end))
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
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r') {
        i += 1;
    }
    i
}

/// Find the byte position of the `\n` that ends the current line (or
/// `bytes.len()` for the last line). The returned position is the index
/// of `\n` itself, not the byte after it.
fn find_line_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
    i
}

/// Match `{` … `}` starting at `open_pos`. Tracks depth, skips over
/// strings (`"..."` with `\"` escapes), character literals, and
/// single-line / block comments. Returns `(inner_text, body_start_byte,
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

/// `(body_start, body_end, past_close_tag, language_id)` where
/// `language_id` is "typescript" (lang="ts" / type="text/typescript") or
/// "javascript" (default).
fn match_script_block(
    bytes: &[u8],
    tag_start: usize,
) -> Option<(usize, usize, usize, &'static str)> {
    if !case_insensitive_prefix(bytes, tag_start, b"<script") { return None; }
    let tag_end = find_byte(bytes, tag_start, b'>')?;
    if bytes.get(tag_end.saturating_sub(1)) == Some(&b'/') { return None; }
    let attr_bytes = &bytes[tag_start..tag_end];
    let language = script_language_from_attrs(attr_bytes);
    let body_start = tag_end + 1;
    let end = find_close_script(bytes, body_start)?;
    Some((body_start, end.0, end.1, language))
}

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

/// Wrap a Razor C# block body in `class __RazorBody { ... }` so
/// tree-sitter-csharp accepts bare field/method declarations.
fn make_csharp_region(
    source: &str,
    byte_start: usize,
    content: &str,
    origin: EmbeddedOrigin,
) -> Option<EmbeddedRegion> {
    if content.is_empty() { return None; }
    let (line, _col) = line_col_at(source.as_bytes(), byte_start);
    let wrapped = format!("class __RazorBody {{\n{content}\n}}");
    Some(EmbeddedRegion {
        language_id: "csharp".to_string(),
        text: wrapped,
        line_offset: line.saturating_sub(1),
        col_offset: 0,
        origin,
        holes: Vec::new(),
        strip_scope_prefix: Some("__RazorBody".to_string()),
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
        strip_scope_prefix: None,
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
        assert!(regions[0].text.contains("var x = 1; var y = 2;"));
        assert!(regions[0].text.contains("class __RazorBody"));
        assert_eq!(regions[0].strip_scope_prefix.as_deref(), Some("__RazorBody"));
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
        let src = "<script>function f() {}</script>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "javascript");
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
        let src = "@* @{ nested } *@\n@{ var x = 1; }";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("var x = 1"));
    }

    #[test]
    fn at_at_escape_is_ignored() {
        let src = "user@@example.com\n@{ var y = 2; }";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("var y = 2"));
    }

    #[test]
    fn strings_inside_block_do_not_terminate_early() {
        let src = "@{ var s = \"}a{\"; var t = 1; }";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("var t = 1"));
    }

    // -----------------------------------------------------------------
    // Directives
    // -----------------------------------------------------------------

    #[test]
    fn model_directive_surfaces_type_as_field() {
        let src = "@model MyApp.Models.Product\n<h1>x</h1>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "csharp");
        assert!(regions[0].text.contains("MyApp.Models.Product __razor_model"));
        assert!(regions[0].text.contains("class __RazorBody"));
    }

    #[test]
    fn inject_directive_surfaces_type_and_name() {
        let src = "@inject IUserService UserSvc\n<h1>x</h1>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("IUserService UserSvc"));
    }

    #[test]
    fn using_directive_emits_using_statement() {
        let src = "@using Microsoft.Extensions.Logging\n<h1>x</h1>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("using Microsoft.Extensions.Logging;"));
        assert!(regions[0].text.contains("class __RazorBody"));
    }

    #[test]
    fn inherits_directive_becomes_base_type() {
        let src = "@inherits RazorPageBase<UserViewModel>\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains(": RazorPageBase<UserViewModel>"));
    }

    #[test]
    fn implements_directive_becomes_interface_list() {
        let src = "@implements IDisposable\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains(": IDisposable"));
    }

    #[test]
    fn namespace_directive_wraps_in_namespace() {
        let src = "@namespace Acme.Web.Views\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("namespace Acme.Web.Views"));
    }

    #[test]
    fn empty_directive_payload_emits_no_region() {
        let src = "@model\n@inject\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn directive_trailing_semicolon_is_stripped() {
        let src = "@using Foo.Bar;\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        // Exactly one semicolon after the using payload (not two).
        let count = regions[0].text.matches("using Foo.Bar;").count();
        assert_eq!(count, 1);
    }

    // -----------------------------------------------------------------
    // Control flow
    // -----------------------------------------------------------------

    #[test]
    fn if_control_flow_produces_method_body() {
        let src = "@if (user.IsAdmin) { <p>Hi @user.Name</p> }";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "csharp");
        assert!(regions[0].text.contains("if (user.IsAdmin)"));
        assert!(regions[0].text.contains("void __M()"));
    }

    #[test]
    fn foreach_control_flow_matched() {
        let src = "@foreach (var item in Model.Items) { <li>@item.Name</li> }";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("foreach (var item in Model.Items)"));
    }

    #[test]
    fn using_with_parens_is_control_flow_not_directive() {
        // `@using (var ctx = new Context()) { ... }` is a disposable
        // using-statement, not a namespace import.
        let src = "@using (var ctx = new Db()) { <p>ok</p> }";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("using (var ctx = new Db())"));
        // Must be wrapped as method body (has `void __M()`), not as a
        // using-directive compilation unit.
        assert!(regions[0].text.contains("void __M()"));
    }

    #[test]
    fn using_without_parens_is_directive() {
        let src = "@using Microsoft.Extensions.Logging\n<h1>x</h1>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        // Namespace-directive shape (no method body wrapper).
        assert!(!regions[0].text.contains("void __M()"));
    }

    #[test]
    fn switch_and_while_and_for_control_flow() {
        let src = "@switch (x) { case 1: break; }\n@while (true) { }\n@for (int i=0;i<10;i++) { }";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 3);
        assert!(regions.iter().all(|r| r.language_id == "csharp"));
        assert!(regions[0].text.contains("switch (x)"));
        assert!(regions[1].text.contains("while (true)"));
        assert!(regions[2].text.contains("for (int i=0;i<10;i++)"));
    }

    // -----------------------------------------------------------------
    // Misc
    // -----------------------------------------------------------------

    #[test]
    fn multiple_constructs_coexist() {
        let src = "@model Foo\n@{ var a = 1; }\n@if (a > 0) { <p>yes</p> }\n<script>alert('hi');</script>";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 4);
        assert_eq!(
            regions.iter().filter(|r| r.language_id == "csharp").count(),
            3
        );
        assert_eq!(
            regions.iter().filter(|r| r.language_id == "javascript").count(),
            1
        );
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
