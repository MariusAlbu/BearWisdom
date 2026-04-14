//! HEEx embedded regions — `{ expr }`, `<%= expr %>`, `<% code %>`.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = crate::languages::common::extract_html_script_style_regions(source);
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // Legacy EEx `<% %>` and `<%= %>`.
        if i + 1 < bytes.len() && bytes[i] == b'<' && bytes[i + 1] == b'%' {
            let kind_b = bytes.get(i + 2).copied();
            if kind_b == Some(b'#') {
                // comment
                if let Some(c) = find_close_pct(bytes, i + 3) { i = c + 2; continue; }
                i += 2; continue;
            }
            let is_expr = matches!(kind_b, Some(b'='));
            let body_start = if is_expr { i + 3 } else { i + 2 };
            let Some(close) = find_close_pct(bytes, body_start) else { i += 2; continue; };
            if let Some(body) = source.get(body_start..close) {
                let t = body.trim();
                if !t.is_empty() {
                    let (line, col) = line_col(bytes, body_start);
                    regions.push(EmbeddedRegion {
                        language_id: "elixir".into(),
                        text: format!("{t}\n"),
                        line_offset: line, col_offset: col,
                        origin: EmbeddedOrigin::TemplateExpr,
                        holes: Vec::new(), strip_scope_prefix: None,
                    });
                }
            }
            i = close + 2;
            continue;
        }
        // `{ expr }` (curly-brace Elixir expression).
        if bytes[i] == b'{' {
            let body_start = i + 1;
            // Find matching `}` (depth-tracked for nested braces in expressions).
            let mut depth = 1; let mut j = body_start;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                if depth == 0 { break; }
                j += 1;
            }
            if j < bytes.len() && depth == 0 {
                if let Some(body) = source.get(body_start..j) {
                    let t = body.trim();
                    // Skip empty or literal values like `class={something}`; we
                    // don't distinguish attribute bindings from prose here.
                    if !t.is_empty() && !t.starts_with('"') && !t.starts_with('\'') {
                        let (line, col) = line_col(bytes, body_start);
                        regions.push(EmbeddedRegion {
                            language_id: "elixir".into(),
                            text: format!("{t}\n"),
                            line_offset: line, col_offset: col,
                            origin: EmbeddedOrigin::TemplateExpr,
                            holes: Vec::new(), strip_scope_prefix: None,
                        });
                    }
                }
                i = j + 1; continue;
            }
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
