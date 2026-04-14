//! Haml embedded-region detection — mirrors Slim with filter syntax.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    let lines: Vec<(u32, &str)> = source.lines().enumerate().map(|(i, l)| (i as u32, l)).collect();
    let mut i = 0usize;
    while i < lines.len() {
        let (line_no, raw) = lines[i];
        let trimmed = raw.trim_start();
        let indent = raw.len() - trimmed.len();
        // Filters like `:javascript`, `:css`, `:ruby`, `:scss`.
        if let Some(filter) = trimmed.strip_prefix(':') {
            let filter = filter.split_whitespace().next().unwrap_or("");
            let (lang, origin) = match filter {
                "javascript" => ("javascript", EmbeddedOrigin::ScriptBlock),
                "css" => ("css", EmbeddedOrigin::StyleBlock),
                "scss" | "sass" => ("scss", EmbeddedOrigin::StyleBlock),
                "ruby" => ("ruby", EmbeddedOrigin::TemplateExpr),
                _ => { i += 1; continue; }
            };
            if let Some(block) = capture_block(&lines, i + 1, indent) {
                regions.push(EmbeddedRegion {
                    language_id: lang.into(),
                    text: block.text, line_offset: block.start, col_offset: 0,
                    origin, holes: Vec::new(), strip_scope_prefix: None,
                });
                i = block.next; continue;
            }
        }
        // Line-level directives.
        if let Some(code) = trimmed.strip_prefix("- ") {
            regions.push(ruby(code, line_no, indent as u32));
        } else if let Some(expr) = trimmed.strip_prefix("= ") {
            regions.push(ruby(expr, line_no, indent as u32));
        } else if let Some(expr) = find_inline_equals_haml(trimmed) {
            regions.push(ruby(expr, line_no, indent as u32));
        } else {
            collect_hash_interp(raw, line_no, &mut regions);
        }
        i += 1;
    }
    regions
}

/// Haml tags start with `%` (`%h1`, `%p`) or `.class` / `#id`. After
/// the tag and optional attrs `(…)`/`{…}`/`[…]`, `=` introduces a
/// Ruby expression.
fn find_inline_equals_haml(trimmed: &str) -> Option<&str> {
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    // Eat leading `%`, `.`, `#` tag selectors.
    while i < bytes.len() && matches!(bytes[i], b'%' | b'.' | b'#') {
        i += 1;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
        { i += 1; }
    }
    if i == 0 { return None; }
    // Optional attr groups: `{...}`, `(...)`, `[...]`.
    while i < bytes.len() && matches!(bytes[i], b'{' | b'(' | b'[') {
        let close = match bytes[i] { b'{' => b'}', b'(' => b')', _ => b']' };
        let mut d = 1; i += 1;
        while i < bytes.len() && d > 0 {
            if bytes[i] == close { d -= 1; }
            i += 1;
            if d == 0 { break; }
        }
    }
    if i < bytes.len() && bytes[i] == b'=' {
        let rest = trimmed.get(i + 1..)?.trim();
        return if rest.is_empty() { None } else { Some(rest) };
    }
    None
}

fn ruby(code: &str, line: u32, col: u32) -> EmbeddedRegion {
    EmbeddedRegion {
        language_id: "ruby".into(),
        text: format!("{}\n", code.trim()),
        line_offset: line, col_offset: col,
        origin: EmbeddedOrigin::TemplateExpr, holes: Vec::new(), strip_scope_prefix: None,
    }
}

struct Block { text: String, start: u32, next: usize }
fn capture_block(lines: &[(u32, &str)], from: usize, opener_indent: usize) -> Option<Block> {
    let mut out = String::new(); let mut start: Option<u32> = None; let mut i = from;
    while i < lines.len() {
        let (ln, raw) = lines[i];
        let t = raw.trim_start();
        if t.is_empty() { out.push('\n'); i += 1; continue; }
        let ind = raw.len() - t.len();
        if ind <= opener_indent { break; }
        if start.is_none() { start = Some(ln); }
        out.push_str(t); out.push('\n'); i += 1;
    }
    Some(Block { text: out, start: start?, next: i })
}

fn collect_hash_interp(line: &str, line_no: u32, regions: &mut Vec<EmbeddedRegion>) {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'#' && bytes[i + 1] == b'{' {
            let start = i + 2;
            let mut depth = 1; let mut j = start;
            while j < bytes.len() && depth > 0 {
                match bytes[j] { b'{' => depth += 1, b'}' => depth -= 1, _ => {} }
                if depth == 0 { break; }
                j += 1;
            }
            if j < bytes.len() && depth == 0 {
                if let Some(text) = line.get(start..j) {
                    let t = text.trim();
                    if !t.is_empty() {
                        regions.push(EmbeddedRegion {
                            language_id: "ruby".into(),
                            text: format!("({t})\n"),
                            line_offset: line_no,
                            col_offset: start as u32,
                            origin: EmbeddedOrigin::TemplateExpr,
                            holes: Vec::new(),
                            strip_scope_prefix: None,
                        });
                    }
                }
                i = j + 1; continue;
            }
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_becomes_ruby() {
        let src = "%h1= @user.name\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "ruby" && r.text.contains("@user")));
    }

    #[test]
    fn js_filter_captures_block() {
        let src = ":javascript\n  console.log(x)\n  const y = 1\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "javascript" && r.text.contains("console.log")));
    }

    #[test]
    fn hash_interpolation_becomes_ruby() {
        let src = "%p Hello #{@user.name}!\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "ruby" && r.text.contains("@user.name")));
    }
}
