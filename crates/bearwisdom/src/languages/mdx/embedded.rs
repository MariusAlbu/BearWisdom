//! MDX embedded-region detection.
//!
//! Produces embedded regions for:
//!
//!   * Fenced code blocks — same dispatch as Markdown (reuses
//!     `markdown::fenced` + `markdown::info_string`).
//!   * YAML / TOML frontmatter — same forms as Markdown.
//!   * ES `import` / `export` statements at the top level —
//!     collected as one TypeScript `ScriptBlock` region so the TS
//!     sub-extractor emits import refs + exported symbols against
//!     the host file.
//!
//! JSX inline component refs are handled by `extract.rs`, not as
//! embedded regions — they become `Calls` refs directly against the
//! host file symbol.

use super::super::markdown::{fenced, info_string};
use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();

    // Frontmatter at top of file.
    let trimmed_start = skip_leading_whitespace(source);
    if let Some(fm) = detect_frontmatter(source, trimmed_start) {
        regions.push(fm);
    }

    // Fenced code blocks — same semantics as Markdown.
    let fences = fenced::parse_fences(source);
    let fence_ranges: Vec<(usize, usize)> = fences
        .iter()
        .map(|f| (f.body_byte_offset, f.body_byte_offset + f.body.len()))
        .collect();
    for fence in &fences {
        let Some(lang) = info_string::normalize(&fence.info) else {
            continue;
        };
        if matches!(lang, "json" | "xml") {
            continue;
        }
        regions.push(EmbeddedRegion {
            language_id: lang.to_string(),
            text: fence.body.clone(),
            line_offset: fence.body_line_offset,
            col_offset: 0,
            origin: EmbeddedOrigin::MarkdownFence,
            holes: Vec::new(),
            strip_scope_prefix: None,
        });
    }

    // Top-level import/export statements → one TS ScriptBlock region.
    if let Some(script) = collect_imports_exports(source, &fence_ranges) {
        regions.push(script);
    }

    regions
}

fn skip_leading_whitespace(source: &str) -> usize {
    let bytes = source.as_bytes();
    let mut i = 0;
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        i = 3;
    }
    while i < bytes.len() && (bytes[i] == b'\n' || bytes[i] == b'\r') {
        i += 1;
    }
    i
}

fn detect_frontmatter(source: &str, start: usize) -> Option<EmbeddedRegion> {
    let bytes = source.as_bytes();
    if start >= bytes.len() {
        return None;
    }
    if let Some(region) = detect_delimited(source, start, "---", "yaml") {
        return Some(region);
    }
    if let Some(region) = detect_delimited(source, start, "+++", "toml") {
        return Some(region);
    }
    None
}

fn detect_delimited(
    source: &str,
    start: usize,
    delim: &str,
    language_id: &'static str,
) -> Option<EmbeddedRegion> {
    let bytes = source.as_bytes();
    let delim_bytes = delim.as_bytes();
    if !starts_on_line(bytes, start, delim_bytes) {
        return None;
    }
    let open_end = find_newline_after(bytes, start + delim_bytes.len())?;
    let body_start = open_end + 1;

    let mut i = body_start;
    while i < bytes.len() {
        let line_end = find_newline_after(bytes, i).unwrap_or(bytes.len());
        let line = source.get(i..line_end)?;
        if line.trim() == delim {
            let body_end = i;
            let body = source.get(body_start..body_end)?.to_string();
            let (line_offset, _) = line_col_at(bytes, body_start);
            return Some(EmbeddedRegion {
                language_id: language_id.to_string(),
                text: body,
                line_offset,
                col_offset: 0,
                origin: EmbeddedOrigin::MarkdownFrontmatter,
                holes: Vec::new(),
                strip_scope_prefix: None,
            });
        }
        i = line_end + 1;
    }
    None
}

fn starts_on_line(bytes: &[u8], start: usize, needle: &[u8]) -> bool {
    if start + needle.len() > bytes.len() {
        return false;
    }
    if &bytes[start..start + needle.len()] != needle {
        return false;
    }
    let after = start + needle.len();
    matches!(bytes.get(after), None | Some(b'\n') | Some(b'\r'))
}

fn find_newline_after(bytes: &[u8], start: usize) -> Option<usize> {
    bytes[start..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|p| start + p)
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

/// Collect all top-level `import`/`export` statements into a single
/// TypeScript `ScriptBlock` region. Lines inside fence ranges are
/// skipped.
///
/// The region's `line_offset` is the line of the FIRST
/// import/export kept; every other statement is concatenated with
/// leading blank lines so its relative line position inside the
/// region matches its position in the host file — this keeps
/// sub-extracted symbol/ref line numbers accurate when spliced back.
fn collect_imports_exports(
    source: &str,
    fence_ranges: &[(usize, usize)],
) -> Option<EmbeddedRegion> {
    let bytes = source.as_bytes();
    let mut line_starts: Vec<usize> = vec![0];
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }

    let mut first_line: Option<u32> = None;
    let mut text = String::new();
    let mut current_line_in_output: u32 = 0;

    let mut line_idx = 0usize;
    while line_idx < line_starts.len() {
        let ls = line_starts[line_idx];
        let le = line_starts.get(line_idx + 1).copied().unwrap_or(bytes.len());
        if inside_any_range(ls, fence_ranges) {
            line_idx += 1;
            continue;
        }
        let line_bytes = &bytes[ls..le];
        if is_import_or_export_start(line_bytes) {
            // Find end of statement: consume balanced braces/parens to
            // handle multi-line imports, stop at a line that ends the
            // statement (semicolon at depth 0 or end of balanced body).
            let (stmt_end_line, stmt_end_byte) =
                find_statement_end(&line_starts, bytes, line_idx);
            let stmt_text = std::str::from_utf8(&bytes[ls..stmt_end_byte])
                .unwrap_or("")
                .trim_end_matches('\r')
                .to_string();
            let stmt_line = line_idx as u32;
            match first_line {
                None => {
                    first_line = Some(stmt_line);
                    text.push_str(&stmt_text);
                    if !text.ends_with('\n') {
                        text.push('\n');
                    }
                    current_line_in_output =
                        stmt_line + (stmt_text.matches('\n').count() as u32) + 1;
                }
                Some(first) => {
                    // Pad with blank lines so this statement sits on
                    // its original line inside the concatenated region.
                    let target_line_in_output = stmt_line - first;
                    while current_line_in_output < target_line_in_output {
                        text.push('\n');
                        current_line_in_output += 1;
                    }
                    text.push_str(&stmt_text);
                    if !text.ends_with('\n') {
                        text.push('\n');
                    }
                    current_line_in_output +=
                        (stmt_text.matches('\n').count() as u32) + 1;
                }
            }
            line_idx = stmt_end_line + 1;
            continue;
        }
        line_idx += 1;
    }

    let first = first_line?;
    Some(EmbeddedRegion {
        language_id: "typescript".to_string(),
        text,
        line_offset: first,
        col_offset: 0,
        origin: EmbeddedOrigin::ScriptBlock,
        holes: Vec::new(),
        strip_scope_prefix: None,
    })
}

fn inside_any_range(pos: usize, ranges: &[(usize, usize)]) -> bool {
    ranges.iter().any(|(s, e)| pos >= *s && pos < *e)
}

fn is_import_or_export_start(line: &[u8]) -> bool {
    // MDX requires import/export at column 0 — no leading whitespace.
    starts_with_word(line, b"import") || starts_with_word(line, b"export")
}

fn starts_with_word(line: &[u8], word: &[u8]) -> bool {
    if line.len() < word.len() + 1 {
        return false;
    }
    if &line[..word.len()] != word {
        return false;
    }
    let next = line[word.len()];
    next == b' ' || next == b'\t' || next == b'{' || next == b'*'
}

/// Given the line index of an import/export statement start, find the
/// line and byte position where the statement ends. Handles multi-line
/// imports by tracking brace/paren balance and returning the line
/// that closes the statement (semicolon or balanced brace).
fn find_statement_end(
    line_starts: &[usize],
    bytes: &[u8],
    start_line: usize,
) -> (usize, usize) {
    let mut depth: i32 = 0;
    let mut in_str: Option<u8> = None;
    let mut escape = false;
    let mut line_idx = start_line;
    while line_idx < line_starts.len() {
        let ls = line_starts[line_idx];
        let le = line_starts.get(line_idx + 1).copied().unwrap_or(bytes.len());
        let mut i = ls;
        while i < le {
            let b = bytes[i];
            if let Some(q) = in_str {
                if escape {
                    escape = false;
                } else if b == b'\\' {
                    escape = true;
                } else if b == q {
                    in_str = None;
                }
            } else {
                match b {
                    b'"' | b'\'' | b'`' => in_str = Some(b),
                    b'{' | b'(' | b'[' => depth += 1,
                    b'}' | b')' | b']' => depth -= 1,
                    b';' if depth == 0 => {
                        return (line_idx, i + 1);
                    }
                    b'\n' if depth == 0 => {
                        // End of line with no pending braces — statement ends here.
                        return (line_idx, i + 1);
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        line_idx += 1;
    }
    (line_starts.len().saturating_sub(1), bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fence_dispatches_like_markdown() {
        let src = "# title\n\n```ts\nexport const x = 1;\n```\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "typescript"
            && r.origin == EmbeddedOrigin::MarkdownFence));
    }

    #[test]
    fn yaml_frontmatter_detected() {
        let src = "---\ntitle: X\n---\n\n# Body\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "yaml"
            && r.origin == EmbeddedOrigin::MarkdownFrontmatter));
    }

    #[test]
    fn import_becomes_script_block() {
        let src = "import { Button } from './button'\n\n# Title\n\n<Button />\n";
        let regions = detect_regions(src);
        let script = regions
            .iter()
            .find(|r| r.origin == EmbeddedOrigin::ScriptBlock)
            .expect("expected a ScriptBlock for imports");
        assert_eq!(script.language_id, "typescript");
        assert!(script.text.contains("import { Button }"));
        assert_eq!(script.line_offset, 0);
    }

    #[test]
    fn export_becomes_script_block() {
        let src = "export const meta = { title: 'Hi' }\n\n# Body\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.origin == EmbeddedOrigin::ScriptBlock
            && r.text.contains("export const meta")));
    }

    #[test]
    fn multiline_import_captured_whole() {
        let src = "import {\n  Button,\n  Card\n} from './ui'\n\n# Title\n";
        let regions = detect_regions(src);
        let script = regions
            .iter()
            .find(|r| r.origin == EmbeddedOrigin::ScriptBlock)
            .unwrap();
        assert!(script.text.contains("Button"));
        assert!(script.text.contains("Card"));
        assert!(script.text.contains("from './ui'"));
    }

    #[test]
    fn multiple_imports_merged_into_one_region() {
        let src = "import A from './a'\nimport B from './b'\n\n# Body\n";
        let regions = detect_regions(src);
        let script_count = regions
            .iter()
            .filter(|r| r.origin == EmbeddedOrigin::ScriptBlock)
            .count();
        assert_eq!(script_count, 1);
        let script = regions
            .iter()
            .find(|r| r.origin == EmbeddedOrigin::ScriptBlock)
            .unwrap();
        assert!(script.text.contains("import A"));
        assert!(script.text.contains("import B"));
    }

    #[test]
    fn import_inside_fence_not_extracted_as_script() {
        let src = "```ts\nimport { x } from 'y'\n```\n\n# After\n";
        let regions = detect_regions(src);
        // Fence dispatches TS region; top-level import scan should find
        // nothing because the import is inside the fence.
        let script_count = regions
            .iter()
            .filter(|r| r.origin == EmbeddedOrigin::ScriptBlock)
            .count();
        assert_eq!(script_count, 0);
    }

    #[test]
    fn empty_source_no_regions() {
        assert!(detect_regions("").is_empty());
    }

    #[test]
    fn frontmatter_and_import_and_fence_coexist() {
        let src = "---\ntitle: X\n---\n\nimport A from './a'\n\n# Body\n\n```ts\nexport const y = 2;\n```\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.origin == EmbeddedOrigin::MarkdownFrontmatter));
        assert!(regions.iter().any(|r| r.origin == EmbeddedOrigin::ScriptBlock));
        assert!(regions.iter().any(|r| r.origin == EmbeddedOrigin::MarkdownFence));
    }
}
