//! Handlebars embedded regions — JS expressions per mustache tag,
//! plus JS/CSS script/style blocks in the surrounding HTML.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = crate::languages::common::extract_html_script_style_regions(source);

    let bytes = source.as_bytes();
    let mut idx = 0u32;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let triple = bytes.get(i + 2).copied() == Some(b'{');
            let expr_start = if triple { i + 3 } else { i + 2 };
            let close_needle_len = if triple { 3 } else { 2 };
            let mut j = expr_start;
            let mut found = None;
            while j + close_needle_len <= bytes.len() {
                if bytes[j] == b'}'
                    && bytes[j + 1] == b'}'
                    && (!triple || bytes[j + 2] == b'}')
                {
                    found = Some(j);
                    break;
                }
                j += 1;
            }
            let Some(expr_end) = found else {
                i += 2;
                continue;
            };
            if let Some(text) = source.get(expr_start..expr_end) {
                let trimmed = text.trim();
                // Skip comments and block open/close markers — no JS there.
                let first = trimmed.chars().next();
                let is_code = !matches!(first, Some('!') | Some('#') | Some('/') | Some('>'));
                if is_code && !trimmed.is_empty() {
                    let (line, col) = line_col_at(bytes, expr_start);
                    regions.push(EmbeddedRegion {
                        language_id: "javascript".to_string(),
                        text: format!(
                            "function __HbsExpr{idx}() {{ return ({trimmed}); }}\n"
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
            i = expr_end + close_needle_len;
            continue;
        }
        i += 1;
    }
    regions
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
        let src = "<p>{{getUserName()}}</p>";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "javascript"
            && r.text.contains("getUserName")));
    }

    #[test]
    fn block_open_close_skipped_as_expressions() {
        let src = "{{#each items}}{{/each}}";
        let regions = detect_regions(src);
        // No JS expressions (open/close markers are structural, not code).
        assert!(regions.iter().all(|r| r.language_id != "javascript"));
    }
}
