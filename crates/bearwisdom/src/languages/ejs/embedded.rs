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
mod tests {
    use super::*;

    #[test]
    fn code_tag_becomes_js_region() {
        let src = "<% const x = getUser(); %>";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.text.contains("getUser")));
    }

    #[test]
    fn equals_tag_becomes_js_region() {
        let src = "<p><%= userName %></p>";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.text.contains("userName")));
    }

    #[test]
    fn raw_tag_becomes_js_region() {
        let src = "<%- renderHtml(body) %>";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.text.contains("renderHtml")));
    }

    #[test]
    fn comment_tag_skipped() {
        let src = "<%# comment content %><p>Hello</p>";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }
}
