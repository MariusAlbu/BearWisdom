//! Nginx embedded regions — OpenResty Lua blocks.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

const LUA_DIRECTIVES: &[&str] = &[
    "content_by_lua_block",
    "access_by_lua_block",
    "init_by_lua_block",
    "init_worker_by_lua_block",
    "log_by_lua_block",
    "header_filter_by_lua_block",
    "body_filter_by_lua_block",
    "rewrite_by_lua_block",
    "balancer_by_lua_block",
    "set_by_lua_block",
];

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // Look for `<directive> {`.
        if let Some((dir_start, dir_end, brace)) = find_lua_directive(bytes, i) {
            let _ = dir_start;
            let _ = dir_end;
            // Extract balanced `{ ... }` body.
            let body_start = brace + 1;
            let mut depth = 1; let mut j = body_start;
            while j < bytes.len() && depth > 0 {
                match bytes[j] { b'{' => depth += 1, b'}' => depth -= 1, _ => {} }
                if depth == 0 { break; }
                j += 1;
            }
            if j < bytes.len() && depth == 0 {
                if let Some(body) = source.get(body_start..j) {
                    let (line, col) = line_col(bytes, body_start);
                    regions.push(EmbeddedRegion {
                        language_id: "lua".into(),
                        text: body.to_string(),
                        line_offset: line, col_offset: col,
                        origin: EmbeddedOrigin::TemplateExpr,
                        holes: Vec::new(), strip_scope_prefix: None,
                    });
                }
                i = j + 1; continue;
            }
        }
        i += 1;
    }
    regions
}

fn find_lua_directive(bytes: &[u8], from: usize) -> Option<(usize, usize, usize)> {
    for dir in LUA_DIRECTIVES {
        let bytes_slice = &bytes[from..];
        let Some(rel) = find_ascii(bytes_slice, dir.as_bytes()) else { continue };
        let start = from + rel;
        // Must be preceded by whitespace or start-of-file.
        if start > 0 && !matches!(bytes[start - 1], b' ' | b'\t' | b'\n' | b'\r') { continue; }
        let end = start + dir.len();
        // Skip whitespace to find opening `{`.
        let mut k = end;
        while k < bytes.len() && matches!(bytes[k], b' ' | b'\t') { k += 1; }
        if k < bytes.len() && bytes[k] == b'{' {
            return Some((start, end, k));
        }
    }
    None
}

fn find_ascii(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() { return None; }
    for i in 0..=haystack.len() - needle.len() {
        if &haystack[i..i + needle.len()] == needle { return Some(i); }
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
