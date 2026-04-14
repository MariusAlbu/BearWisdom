//! Mako embedded regions — `${expr}` and `<% code %>` → Python.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = crate::languages::common::extract_html_script_style_regions(source);
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // ${expr}
        if i + 1 < bytes.len() && bytes[i] == b'$' && bytes[i + 1] == b'{' {
            let start = i + 2;
            let mut depth = 1; let mut j = start;
            while j < bytes.len() && depth > 0 {
                match bytes[j] { b'{' => depth += 1, b'}' => depth -= 1, _ => {} }
                if depth == 0 { break; }
                j += 1;
            }
            if j < bytes.len() && depth == 0 {
                if let Some(text) = source.get(start..j) {
                    let t = text.trim();
                    if !t.is_empty() {
                        let (line, col) = line_col(bytes, start);
                        regions.push(EmbeddedRegion {
                            language_id: "python".into(),
                            text: format!("({t})\n"),
                            line_offset: line, col_offset: col,
                            origin: EmbeddedOrigin::TemplateExpr,
                            holes: Vec::new(), strip_scope_prefix: None,
                        });
                    }
                }
                i = j + 1; continue;
            }
        }
        // <% code %>
        if i + 1 < bytes.len() && bytes[i] == b'<' && bytes[i + 1] == b'%' {
            let body_start = if bytes.get(i + 2).copied() == Some(b'!') { i + 3 } else { i + 2 };
            // skip `<%def`, `<%include`, `<%inherit`, `<%doc>`.
            if body_start + 3 < bytes.len() {
                let head = &bytes[body_start..body_start + 3];
                if head.starts_with(b"def") || head.starts_with(b"inc") || head.starts_with(b"inh")
                    || head.starts_with(b"doc")
                { i += 2; continue; }
            }
            let Some(close) = find_close_pct(bytes, body_start) else { i += 2; continue; };
            if let Some(body) = source.get(body_start..close) {
                let t = body.trim();
                if !t.is_empty() {
                    let (line, col) = line_col(bytes, body_start);
                    regions.push(EmbeddedRegion {
                        language_id: "python".into(),
                        text: format!("{t}\n"),
                        line_offset: line, col_offset: col,
                        origin: EmbeddedOrigin::TemplateExpr,
                        holes: Vec::new(), strip_scope_prefix: None,
                    });
                }
            }
            i = close + 2; continue;
        }
        i += 1;
    }
    regions
}

fn find_close_pct(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'%' && bytes[i + 1] == b'>' { return Some(i); }
        i += 1;
    }
    None
}

fn line_col(bytes: &[u8], pos: usize) -> (u32, u32) {
    let mut line: u32 = 0; let mut nl: usize = 0;
    for (i, b) in bytes.iter().enumerate().take(pos) {
        if *b == b'\n' { line += 1; nl = i + 1; }
    }
    (line, (pos - nl) as u32)
}
