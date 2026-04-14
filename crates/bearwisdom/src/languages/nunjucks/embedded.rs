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
                    let (line, col) = line_col_at(bytes, body_start);
                    regions.push(EmbeddedRegion {
                        language_id: "javascript".to_string(),
                        text: format!(
                            "function __NjkExpr{idx}() {{ return ({trimmed}); }}\n"
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
    fn expression_becomes_js_region() {
        let src = "<p>{{ currentUser.name }}</p>";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.text.contains("currentUser.name")));
    }
}
