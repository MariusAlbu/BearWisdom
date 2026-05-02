//! Nunjucks embedded regions — `{{ expr }}` becomes a JS expression.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = crate::languages::common::extract_html_script_style_regions(source);
    let bytes = source.as_bytes();
    let mut idx = 0u32;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let body_start = i + 2;
            let Some(close) = find_double_close(bytes, body_start) else {
                i += 2;
                continue;
            };
            if let Some(body) = source.get(body_start..close) {
                let trimmed = body.trim().trim_start_matches('-').trim_end_matches('-').trim();
                if !trimmed.is_empty() {
                    // Jinja/Nunjucks pipe-filter syntax (`x | upper`,
                    // `x | indent(4)`) doesn't survive a literal JS embed —
                    // the JS extractor emits each filter name as a Calls
                    // ref, polluting the index with hundreds of false
                    // positives (`indent`, `to_nice_yaml`, `regex_replace`,
                    // ...) that have no defining symbols. Strip the filter
                    // chain at the first top-level `|` and embed only the
                    // leading expression. We lose visibility into refs
                    // hidden inside filter arguments; that's a follow-up.
                    let expr = strip_pipe_filters(trimmed);
                    if expr.is_empty() {
                        i = close + 2;
                        continue;
                    }
                    let (line, col) = line_col_at(bytes, body_start);
                    regions.push(EmbeddedRegion {
                        language_id: "javascript".to_string(),
                        text: format!(
                            "function __NjkExpr{idx}() {{ return ({expr}); }}\n"
                        ),
                        line_offset: line,
                        col_offset: col,
                        origin: EmbeddedOrigin::TemplateExpr,
                        holes: Vec::new(),
                        strip_scope_prefix: None,
                    });
                    idx += 1;
                }
            }
            i = close + 2;
            continue;
        }
        i += 1;
    }
    regions
}

fn find_double_close(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'}' && bytes[i + 1] == b'}' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Truncate a Nunjucks/Jinja `{{ }}` body at the first top-level `|`
/// (filter operator). Skips `|` inside parens, brackets, or string literals
/// so legitimate JS-shaped pipes (e.g. inside arg lists) aren't clipped
/// prematurely. Trailing whitespace from the expression is trimmed.
fn strip_pipe_filters(body: &str) -> &str {
    let bytes = body.as_bytes();
    let mut depth: i32 = 0;
    let mut in_str: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(quote) = in_str {
            if b == b'\\' { i += 2; continue }
            if b == quote { in_str = None; }
            i += 1;
            continue;
        }
        match b {
            b'"' | b'\'' => in_str = Some(b),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'|' if depth == 0 => {
                // Don't clip on `||` (logical-or) — Jinja's filter is a
                // single bar.
                if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                    i += 2;
                    continue;
                }
                return body[..i].trim_end();
            }
            _ => {}
        }
        i += 1;
    }
    body.trim_end()
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
pub(super) fn _test_strip_pipe_filters(body: &str) -> &str {
    strip_pipe_filters(body)
}
