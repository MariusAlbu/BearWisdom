//! ERB embedded-region detection. `<%= expr %>` and `<% code %>`
//! produce Ruby regions.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = crate::languages::common::extract_html_script_style_regions(source);
    let bytes = source.as_bytes();
    let mut idx = 0u32;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'%' {
            let kind_byte = bytes.get(i + 2).copied();
            if kind_byte == Some(b'#') {
                if let Some(end) = find_close(bytes, i + 3) { i = end + 2; continue; }
                i += 2; continue;
            }
            let is_expr = matches!(kind_byte, Some(b'=') | Some(b'-'));
            let body_start = if is_expr { i + 3 } else { i + 2 };
            let Some(close) = find_close(bytes, body_start) else { i += 2; continue; };
            // Trim trailing `-` before `%>`.
            let mut body_end = close;
            if body_end > body_start && bytes[body_end - 1] == b'-' { body_end -= 1; }
            if let Some(body) = source.get(body_start..body_end) {
                let trimmed = body.trim();
                if !trimmed.is_empty() {
                    let (line, col) = line_col_at(bytes, body_start);
                    regions.push(EmbeddedRegion {
                        language_id: "ruby".to_string(),
                        text: format!("{trimmed}\n"),
                        line_offset: line,
                        col_offset: col,
                        origin: EmbeddedOrigin::TemplateExpr,
                        holes: Vec::new(),
                        strip_scope_prefix: None,
                    });
                    idx += 1;
                }
            }
            let _ = idx;
            i = close + 2;
            continue;
        }
        i += 1;
    }
    regions
}

fn find_close(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'%' && bytes[i + 1] == b'>' { return Some(i); }
        i += 1;
    }
    None
}

fn line_col_at(bytes: &[u8], byte_pos: usize) -> (u32, u32) {
    let mut line: u32 = 0; let mut last_nl: usize = 0;
    for (i, b) in bytes.iter().enumerate().take(byte_pos) {
        if *b == b'\n' { line += 1; last_nl = i + 1; }
    }
    (line, (byte_pos - last_nl) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equals_tag_becomes_ruby_region() {
        let src = "<p><%= @user.name %></p>";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "ruby" && r.text.contains("@user.name")));
    }

    #[test]
    fn code_tag_becomes_ruby_region() {
        let src = "<% if current_user %>hi<% end %>";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.text.contains("current_user")));
    }

    #[test]
    fn trim_variant_handled() {
        let src = "<%- items.each do |x| -%>\n<%= x %>\n<%- end -%>";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.text.contains("items.each")));
    }

    #[test]
    fn comment_skipped() {
        let src = "<%# just a comment %><p>x</p>";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }
}
