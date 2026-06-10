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

use super::embedded_mask::mask_razor_expressions_in_script;
use super::embedded_scan::{
    find_line_end, find_subseq, has_prefix, line_col_at, make_csharp_region, make_region,
    match_brace_block, match_paren_block, match_script_block, skip_ascii_ws,
};

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
                if let Some(r) = region {
                    regions.push(r);
                }
                i = end;
                continue;
            }
            if let Some((region, end)) = try_directive(source, bytes, i) {
                if let Some(r) = region {
                    regions.push(r);
                }
                i = end;
                continue;
            }
            if let Some((region, end)) = try_code_or_functions(source, bytes, i) {
                if let Some(r) = region {
                    regions.push(r);
                }
                i = end;
                continue;
            }
            if let Some((region, end)) = try_at_brace(source, bytes, i) {
                if let Some(r) = region {
                    regions.push(r);
                }
                i = end;
                continue;
            }
            if let Some((region, end)) = try_at_paren(source, bytes, i) {
                if let Some(r) = region {
                    regions.push(r);
                }
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
                        // Razor expressions like `@Html.Raw(string.Join(...))`
                        // inside `<script>` blocks are server-side C# that
                        // Razor substitutes before the page ships. If passed
                        // as-is to the JS extractor they surface as ghost
                        // JS type refs (`Html`, `Config`, `Model`, …). Mask
                        // each Razor construct with same-width whitespace so
                        // the JS parser ignores them while byte offsets and
                        // line positions stay accurate.
                        let masked = mask_razor_expressions_in_script(content);
                        if let Some(region) = make_region(
                            source,
                            body_start,
                            &masked,
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
    if bytes.get(paren_pos) != Some(&b'(') {
        return None;
    }
    let (cond, _cond_body_start, after_cond) = match_paren_block(bytes, paren_pos)?;

    let brace_pos = skip_ascii_ws(bytes, after_cond);
    if bytes.get(brace_pos) != Some(&b'{') {
        return None;
    }
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
    if !has_prefix(bytes, kw_start, b"using") {
        return None;
    }
    let after = kw_start + 5;
    let peek = skip_ascii_ws(bytes, after);
    if bytes.get(peek) == Some(&b'(') {
        Some(("using", after))
    } else {
        None
    }
}

/// Try a list of keywords; return the one that matches plus the byte
/// position immediately after it. Checks word boundary to avoid matching
/// `@ifable`.
fn match_keyword<'a>(
    bytes: &[u8],
    at: usize,
    keywords: &'a [&'a [u8]],
) -> Option<(&'a str, usize)> {
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
fn try_directive(source: &str, bytes: &[u8], at: usize) -> Option<(Option<EmbeddedRegion>, usize)> {
    // Keywords ordered so longer prefixes win (e.g. `implements` before
    // a hypothetical `imp`). `using` comes AFTER the control-flow check
    // in the caller so `@using (x) { }` doesn't land here.
    static DIRECTIVES: &[&[u8]] = &[
        b"model",
        b"inject",
        b"inherits",
        b"implements",
        b"using",
        b"namespace",
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
        "namespace" => format!("namespace {payload} {{ class __RazorBody {{}} }}"),
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
    if bytes.get(brace_pos) != Some(&b'{') {
        return None;
    }
    let (content, body_start, end) = match_brace_block(bytes, brace_pos)?;
    let region = make_csharp_region(source, body_start, content, EmbeddedOrigin::RazorCode);
    Some((region, end))
}

/// `@{ ... }`.
fn try_at_brace(source: &str, bytes: &[u8], at: usize) -> Option<(Option<EmbeddedRegion>, usize)> {
    if bytes.get(at + 1) != Some(&b'{') {
        return None;
    }
    let (content, body_start, end) = match_brace_block(bytes, at + 1)?;
    let region = make_csharp_region(source, body_start, content, EmbeddedOrigin::RazorCode);
    Some((region, end))
}

/// `@(expr)`.
fn try_at_paren(source: &str, bytes: &[u8], at: usize) -> Option<(Option<EmbeddedRegion>, usize)> {
    if bytes.get(at + 1) != Some(&b'(') {
        return None;
    }
    let (content, body_start, end) = match_paren_block(bytes, at + 1)?;
    let region = make_csharp_region(source, body_start, content, EmbeddedOrigin::RazorCode);
    Some((region, end))
}
