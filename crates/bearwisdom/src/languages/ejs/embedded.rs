//! EJS embedded-region detection.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = crate::languages::common::extract_html_script_style_regions(source);
    let bytes = source.as_bytes();
    let mut idx = 0u32;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'%' {
            // Literal `<%%` → skip.
            if bytes.get(i + 2).copied() == Some(b'%') {
                i += 3;
                continue;
            }
            let kind = bytes.get(i + 2).copied();
            // Skip comments.
            if kind == Some(b'#') {
                if let Some(end) = find_close_percent(bytes, i + 3) {
                    i = end + 2;
                    continue;
                }
                i += 2;
                continue;
            }
            let (body_start, is_expr) = match kind {
                Some(b'=') | Some(b'-') => (i + 3, true),
                _ => (i + 2, false),
            };
            let Some(close_start) = find_close_percent(bytes, body_start) else {
                i += 2;
                continue;
            };
            if let Some(body) = source.get(body_start..close_start) {
                let trimmed = body.trim();
                if !trimmed.is_empty() {
                    // Replace `include('path')` with `null` so the embedded
                    // JS extractor doesn't emit each include as an
                    // unresolvable `include` Calls ref. The host-level EJS
                    // extractor captures these as Imports refs that
                    // EjsResolver matches against the partial file's
                    // class symbol — a single, correct edge per partial.
                    let trimmed = strip_include_calls(trimmed);
                    let trimmed = trimmed.trim();
                    if trimmed.is_empty() {
                        i = close_start + 2;
                        continue;
                    }
                    let (line, col) = line_col_at(bytes, body_start);
                    let text = if is_expr {
                        format!("function __EjsExpr{idx}() {{ return ({trimmed}); }}\n")
                    } else {
                        format!("function __EjsCode{idx}() {{ {trimmed} }}\n")
                    };
                    regions.push(EmbeddedRegion {
                        language_id: "javascript".to_string(),
                        text,
                        line_offset: line,
                        col_offset: col,
                        origin: EmbeddedOrigin::TemplateExpr,
                        holes: Vec::new(),
                        strip_scope_prefix: None,
                    });
                    idx += 1;
                }
            }
            i = close_start + 2;
            continue;
        }
        i += 1;
    }
    regions
}

/// Replace every `include('path')` / `include("path")` call in the body
/// with the literal `null`. The path arguments live in the host-level
/// extractor's Imports refs; here we just want them out of the JS so
/// the JS extractor doesn't see `include` as a function call.
fn strip_include_calls(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // Look for `include(` at an identifier boundary.
        if i + 8 <= bytes.len() && &bytes[i..i + 8] == b"include(" {
            let boundary_ok = i == 0 || {
                let prev = bytes[i - 1];
                !(prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$')
            };
            if boundary_ok {
                // Walk forward through balanced parens, skipping string
                // literals, until the matching `)`.
                let mut depth: i32 = 1;
                let mut j = i + 8;
                let mut in_str: Option<u8> = None;
                while j < bytes.len() && depth > 0 {
                    let c = bytes[j];
                    if let Some(q) = in_str {
                        if c == b'\\' && j + 1 < bytes.len() {
                            j += 2;
                            continue;
                        }
                        if c == q {
                            in_str = None;
                        }
                        j += 1;
                        continue;
                    }
                    match c {
                        b'\'' | b'"' | b'`' => in_str = Some(c),
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                out.push_str("null");
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Find `%>` starting at byte index `from`. Treats `%%>` as literal
/// (not a close). Returns index of `%`.
fn find_close_percent(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'%' && bytes[i + 1] == b'>' {
            // Literal `%%>` is two percents followed by `>`; we want
            // the FIRST percent of `%>` to NOT be preceded by another `%`.
            if i > 0 && bytes[i - 1] == b'%' {
                i += 1;
                continue;
            }
            return Some(i);
        }
        i += 1;
    }
    None
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
#[path = "embedded_tests.rs"]
mod tests;

#[cfg(test)]
pub(super) fn _test_strip_include_calls(body: &str) -> String {
    strip_include_calls(body)
}
