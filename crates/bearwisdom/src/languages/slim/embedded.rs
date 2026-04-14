//! Slim embedded-region detection.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    let lines: Vec<(u32, &str)> = source.lines().enumerate().map(|(i, l)| (i as u32, l)).collect();
    let mut i = 0usize;
    while i < lines.len() {
        let (line_no, raw) = lines[i];
        let trimmed = raw.trim_start();
        let indent = raw.len() - trimmed.len();
        if let Some(expr) = trimmed.strip_prefix("== ") {
            regions.push(make(expr, line_no, indent as u32, "ruby"));
        } else if let Some(expr) = trimmed.strip_prefix("= ") {
            regions.push(make(expr, line_no, indent as u32, "ruby"));
        } else if let Some(expr) = find_inline_equals(trimmed) {
            regions.push(make(expr, line_no, indent as u32, "ruby"));
        } else if let Some(code) = trimmed.strip_prefix("- ") {
            regions.push(make_stmt(code, line_no, indent as u32, "ruby"));
        } else if trimmed == "ruby:" {
            if let Some(block) = capture_block(&lines, i + 1, indent) {
                regions.push(EmbeddedRegion {
                    language_id: "ruby".into(),
                    text: block.text, line_offset: block.start, col_offset: 0,
                    origin: EmbeddedOrigin::TemplateExpr, holes: Vec::new(), strip_scope_prefix: None,
                });
                i = block.next; continue;
            }
        } else if trimmed == "javascript:" {
            if let Some(block) = capture_block(&lines, i + 1, indent) {
                regions.push(EmbeddedRegion {
                    language_id: "javascript".into(),
                    text: block.text, line_offset: block.start, col_offset: 0,
                    origin: EmbeddedOrigin::ScriptBlock, holes: Vec::new(), strip_scope_prefix: None,
                });
                i = block.next; continue;
            }
        } else if trimmed == "css:" || trimmed == "scss:" {
            let lang = if trimmed == "scss:" { "scss" } else { "css" };
            if let Some(block) = capture_block(&lines, i + 1, indent) {
                regions.push(EmbeddedRegion {
                    language_id: lang.into(),
                    text: block.text, line_offset: block.start, col_offset: 0,
                    origin: EmbeddedOrigin::StyleBlock, holes: Vec::new(), strip_scope_prefix: None,
                });
                i = block.next; continue;
            }
        }
        i += 1;
    }
    regions
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

fn find_inline_equals(trimmed: &str) -> Option<&str> {
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric()
            || bytes[i] == b'-' || bytes[i] == b'_'
            || bytes[i] == b'.' || bytes[i] == b'#')
    { i += 1; }
    if i == 0 { return None; }
    if i < bytes.len() && bytes[i] == b'(' {
        let mut d = 1; i += 1;
        while i < bytes.len() && d > 0 {
            match bytes[i] { b'(' => d += 1, b')' => d -= 1, _ => {} }
            i += 1;
            if d == 0 { break; }
        }
    }
    if i + 1 < bytes.len() && bytes[i] == b'=' && bytes[i + 1] == b'=' {
        let rest = trimmed.get(i + 2..)?.trim();
        return if rest.is_empty() { None } else { Some(rest) };
    }
    if i < bytes.len() && bytes[i] == b'=' {
        let rest = trimmed.get(i + 1..)?.trim();
        return if rest.is_empty() { None } else { Some(rest) };
    }
    None
}

fn make(expr: &str, line: u32, col: u32, lang: &str) -> EmbeddedRegion {
    EmbeddedRegion {
        language_id: lang.into(),
        text: format!("({})\n", expr.trim()),
        line_offset: line, col_offset: col,
        origin: EmbeddedOrigin::TemplateExpr, holes: Vec::new(), strip_scope_prefix: None,
    }
}
fn make_stmt(code: &str, line: u32, col: u32, lang: &str) -> EmbeddedRegion {
    EmbeddedRegion {
        language_id: lang.into(),
        text: format!("{}\n", code.trim()),
        line_offset: line, col_offset: col,
        origin: EmbeddedOrigin::TemplateExpr, holes: Vec::new(), strip_scope_prefix: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_becomes_ruby_region() {
        let src = "h1= @user.name\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "ruby" && r.text.contains("@user")));
    }

    #[test]
    fn dash_code_becomes_ruby_region() {
        let src = "- @user = current_user\nh1= @user.name\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.text.contains("current_user")));
    }

    #[test]
    fn javascript_block_captured() {
        let src = "javascript:\n  console.log(hi)\n  const x = 1\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "javascript" && r.text.contains("console.log")));
    }
}
