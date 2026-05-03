//! Pug embedded regions.
//!
//! Each `- code` line and each `= expr` / `!= expr` becomes a small
//! JavaScript region. `#{expr}` interpolations are detected inline.
//! `script.` and `style.` indented blocks capture everything below
//! until indentation drops.

use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    let mut idx = 0u32;

    let mut lines: Vec<(u32, &str)> = Vec::new();
    for (i, l) in source.lines().enumerate() {
        lines.push((i as u32, l));
    }
    let mut line_idx = 0usize;
    while line_idx < lines.len() {
        let (line_no, raw) = lines[line_idx];
        let trimmed = raw.trim_start();
        let indent = raw.len() - trimmed.len();

        if let Some(code) = trimmed.strip_prefix("- ") {
            regions.push(make_js(code, line_no, indent as u32, idx, false));
            idx += 1;
        } else if let Some(expr) = trimmed.strip_prefix("!= ") {
            regions.push(make_js(expr, line_no, indent as u32, idx, true));
            idx += 1;
        } else if let Some(expr) = trimmed.strip_prefix("= ") {
            regions.push(make_js(expr, line_no, indent as u32, idx, true));
            idx += 1;
        } else if let Some(expr) = find_inline_equals_expr(trimmed) {
            // `tagname= expr` or `tag(attrs)= expr` — everything after
            // the first `=` (when preceded by a non-space, non-equals
            // char) is the JS expression.
            regions.push(make_js(expr, line_no, indent as u32, idx, true));
            idx += 1;
        } else if trimmed == "script." || trimmed.starts_with("script(") && trimmed.ends_with('.')
        {
            if let Some(block) = capture_indented_block(&lines, line_idx + 1, indent) {
                regions.push(EmbeddedRegion {
                    language_id: "javascript".to_string(),
                    text: block.text,
                    line_offset: block.start_line,
                    col_offset: 0,
                    origin: EmbeddedOrigin::ScriptBlock,
                    holes: Vec::new(),
                    strip_scope_prefix: None,
                });
                line_idx = block.next_line_idx;
                continue;
            }
        } else if trimmed == "style." || trimmed.starts_with("style(") && trimmed.ends_with('.') {
            if let Some(block) = capture_indented_block(&lines, line_idx + 1, indent) {
                regions.push(EmbeddedRegion {
                    language_id: "css".to_string(),
                    text: block.text,
                    line_offset: block.start_line,
                    col_offset: 0,
                    origin: EmbeddedOrigin::StyleBlock,
                    holes: Vec::new(),
                    strip_scope_prefix: None,
                });
                line_idx = block.next_line_idx;
                continue;
            }
        }

        // Inline `#{expr}` interpolations on any line.
        collect_hash_interpolations(raw, line_no, &mut idx, &mut regions);

        line_idx += 1;
    }

    regions
}

struct IndentedBlock {
    text: String,
    start_line: u32,
    next_line_idx: usize,
}

fn capture_indented_block(
    lines: &[(u32, &str)],
    start_idx: usize,
    opener_indent: usize,
) -> Option<IndentedBlock> {
    if start_idx >= lines.len() {
        return None;
    }
    let mut out = String::new();
    let mut start_line: Option<u32> = None;
    let mut i = start_idx;
    while i < lines.len() {
        let (line_no, raw) = lines[i];
        let trimmed = raw.trim_start();
        if trimmed.is_empty() {
            out.push('\n');
            i += 1;
            continue;
        }
        let indent = raw.len() - trimmed.len();
        if indent <= opener_indent {
            break;
        }
        if start_line.is_none() {
            start_line = Some(line_no);
        }
        out.push_str(trimmed);
        out.push('\n');
        i += 1;
    }
    let start_line = start_line?;
    Some(IndentedBlock {
        text: out,
        start_line,
        next_line_idx: i,
    })
}

/// Look for `tag= expr` or `tag(attrs)= expr` patterns where `=` sits
/// immediately after a tag/attr-list segment. Returns the trimmed
/// expression text.
fn find_inline_equals_expr(trimmed: &str) -> Option<&str> {
    // Skip the leading tag name (letters/digits/hyphen/underscore/dot/#).
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric()
            || bytes[i] == b'-'
            || bytes[i] == b'_'
            || bytes[i] == b'.'
            || bytes[i] == b'#')
    {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    // Optional `(attrs...)` — skip balanced parens.
    if i < bytes.len() && bytes[i] == b'(' {
        let mut depth = 1;
        i += 1;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            i += 1;
            if depth == 0 {
                break;
            }
        }
    }
    // Now expect `= expr` or `!= expr`.
    let mut raw = false;
    if i + 1 < bytes.len() && bytes[i] == b'!' && bytes[i + 1] == b'=' {
        raw = true;
        i += 2;
    } else if i < bytes.len() && bytes[i] == b'=' {
        i += 1;
    } else {
        return None;
    }
    let _ = raw;
    // Expression is everything after, trimmed.
    let rest = trimmed.get(i..)?.trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}

// View-runtime locals every Pug embedded region inherits from the host
// rendering layer. Express + the `i18n` npm package patches `res.locals`
// with `__`, `__n`, `__l`, `__mf`, `__h` before the template is rendered,
// so Pug expressions like `#{__('Welcome')}` reference helpers that are
// in scope at render time but never appear as importable identifiers in
// the project. Declaring stub bindings inside the wrapper function gives
// the JS resolver a same-scope local to bind the call to, suppressing
// the unresolved-Calls noise without hand-maintaining a builtin list.
const VIEW_RUNTIME_LOCALS: &str =
    "var __ = (s) => s; var __n = (s,p,n) => s; var __l = (s) => [s]; \
     var __mf = (s,v) => s; var __h = (s) => s;";

fn make_js(expr: &str, line_no: u32, col: u32, idx: u32, wrap_return: bool) -> EmbeddedRegion {
    let code = expr.trim();
    let text = if wrap_return {
        format!("function __PugExpr{idx}() {{ {VIEW_RUNTIME_LOCALS} return ({code}); }}\n")
    } else {
        format!("function __PugCode{idx}() {{ {VIEW_RUNTIME_LOCALS} {code} }}\n")
    };
    EmbeddedRegion {
        language_id: "javascript".to_string(),
        text,
        line_offset: line_no,
        col_offset: col,
        origin: EmbeddedOrigin::TemplateExpr,
        holes: Vec::new(),
        strip_scope_prefix: None,
    }
}

fn collect_hash_interpolations(
    line: &str,
    line_no: u32,
    idx: &mut u32,
    regions: &mut Vec<EmbeddedRegion>,
) {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'#' && bytes[i + 1] == b'{' {
            let start = i + 2;
            let mut depth = 1;
            let mut j = start;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                if depth == 0 {
                    break;
                }
                j += 1;
            }
            if j < bytes.len() && depth == 0 {
                if let Some(text) = line.get(start..j) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        regions.push(EmbeddedRegion {
                            language_id: "javascript".to_string(),
                            text: format!(
                                "function __PugInterp{idx}() {{ {VIEW_RUNTIME_LOCALS} return ({trimmed}); }}\n",
                                idx = *idx
                            ),
                            line_offset: line_no,
                            col_offset: start as u32,
                            origin: EmbeddedOrigin::TemplateExpr,
                            holes: Vec::new(),
                            strip_scope_prefix: None,
                        });
                        *idx += 1;
                    }
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
}

#[cfg(test)]
#[path = "embedded_tests.rs"]
mod tests;
