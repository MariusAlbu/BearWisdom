//! Low-level byte scanners for the Razor region detector — brace/paren
//! matching, C# string/comment skipping, `<script>` open/close detection,
//! and region builders shared by the construct dispatchers in
//! `super::embedded`.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub(super) fn has_prefix(bytes: &[u8], start: usize, needle: &[u8]) -> bool {
    if start >= bytes.len() { return false; }
    bytes[start..].starts_with(needle)
}

pub(super) fn find_subseq(bytes: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || start > bytes.len() { return None; }
    let end = bytes.len().saturating_sub(needle.len()) + 1;
    (start..end).find(|&i| bytes[i..].starts_with(needle))
}

pub(super) fn skip_ascii_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r') {
        i += 1;
    }
    i
}

/// Find the byte position of the `\n` that ends the current line (or
/// `bytes.len()` for the last line). The returned position is the index
/// of `\n` itself, not the byte after it.
pub(super) fn find_line_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
    i
}

/// Match `{` … `}` starting at `open_pos`. Tracks depth, skips over
/// strings (`"..."` with `\"` escapes), character literals, and
/// single-line / block comments. Returns `(inner_text, body_start_byte,
/// past_closing_brace_byte)`.
pub(super) fn match_brace_block(bytes: &[u8], open_pos: usize) -> Option<(&str, usize, usize)> {
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

pub(super) fn match_paren_block(bytes: &[u8], open_pos: usize) -> Option<(&str, usize, usize)> {
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
pub(super) fn match_script_block(
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
pub(super) fn make_csharp_region(
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

pub(super) fn make_region(
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

pub(super) fn line_col_at(bytes: &[u8], byte_pos: usize) -> (u32, u32) {
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
