//! Go template embedded regions — `{{ expr }}` dispatched as Go.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = crate::languages::common::extract_html_script_style_regions(source);
    let bytes = source.as_bytes();
    let mut idx = 0u32;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let body_start = i + 2;
            let Some(close) = find_close(bytes, body_start) else { i += 2; continue; };
            if let Some(body) = source.get(body_start..close) {
                let t = body.trim().trim_start_matches('-').trim_end_matches('-').trim();
                // Skip comments (leading /*).
                if t.starts_with("/*") { i = close + 2; continue; }
                // Skip structural keywords — pure Go identifier expressions go to Go extractor.
                let first = t.split_whitespace().next().unwrap_or("");
                if matches!(first, "end" | "else") { i = close + 2; continue; }
                if !t.is_empty() {
                    let (line, col) = line_col(bytes, body_start);
                    regions.push(EmbeddedRegion {
                        language_id: "go".into(),
                        text: format!("package __gt\nfunc __GtExpr{idx}() interface{{}} {{ return nil }}\n"),
                        line_offset: line, col_offset: col,
                        origin: EmbeddedOrigin::TemplateExpr,
                        holes: Vec::new(), strip_scope_prefix: None,
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

fn find_close(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'}' && bytes[i + 1] == b'}' { return Some(i); }
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
