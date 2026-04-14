//! Markdown embedded-region detection.
//!
//! Produces one `EmbeddedRegion` per:
//!
//!   * Fenced code block whose info-string normalizes to a known
//!     language id — the body is dispatched to that language's plugin
//!     with `origin = MarkdownFence` so downstream stats can flag
//!     snippet-origin unresolved refs.
//!
//!   * Frontmatter block at the top of the file:
//!       * `---\n...\n---`  → YAML
//!       * `+++\n...\n+++`  → TOML
//!       * `{\n...\n}` at BOF (Hexo-style) → JSON
//!     `origin = MarkdownFrontmatter` — these regions are NOT
//!     snippet-tagged; frontmatter is structured configuration.

use super::fenced;
use super::info_string;
use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();

    // Frontmatter must appear at the very start of the file (optional
    // BOM / initial newlines are tolerated).
    let trimmed_start = skip_leading_whitespace(source);
    if let Some(fm) = detect_frontmatter(source, trimmed_start) {
        regions.push(fm);
    }

    // Fenced code blocks.
    for fence in fenced::parse_fences(source) {
        let Some(lang) = info_string::normalize(&fence.info) else {
            continue;
        };
        // JSON/YAML/TOML/XML plugins have no extractor but also produce
        // no noise — skipping them here saves work. Still emit regions
        // for yaml/toml when they're in frontmatter (above), but not
        // inside ordinary fenced blocks (they rarely hold refs).
        if matches!(lang, "json" | "xml") {
            continue;
        }
        regions.push(EmbeddedRegion {
            language_id: lang.to_string(),
            text: fence.body,
            line_offset: fence.body_line_offset,
            col_offset: 0,
            origin: EmbeddedOrigin::MarkdownFence,
            holes: Vec::new(),
            strip_scope_prefix: None,
        });
    }
    regions
}

fn skip_leading_whitespace(source: &str) -> usize {
    let bytes = source.as_bytes();
    let mut i = 0;
    // BOM
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
    // YAML: --- ... ---
    if let Some(region) = detect_delimited(source, start, "---", "yaml") {
        return Some(region);
    }
    // TOML: +++ ... +++
    if let Some(region) = detect_delimited(source, start, "+++", "toml") {
        return Some(region);
    }
    // Hexo-style JSON: the file begins with `{` (balanced by a `}` on a
    // line by itself). Narrow: require the match to be within the first
    // ~4KB of the file.
    if bytes[start] == b'{' {
        if let Some(end) = find_toplevel_brace_close(source, start) {
            let body_start = start + 1;
            let body_end = end;
            let (line_offset, _) = line_col_at(bytes, body_start);
            return Some(EmbeddedRegion {
                language_id: "json".to_string(),
                text: format!("{{{}}}", source.get(body_start..body_end).unwrap_or("")),
                line_offset,
                col_offset: 0,
                origin: EmbeddedOrigin::MarkdownFrontmatter,
                holes: Vec::new(),
                strip_scope_prefix: None,
            });
        }
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
    // Opening line ends at next newline.
    let open_end = find_newline_after(bytes, start + delim_bytes.len())?;
    let body_start = open_end + 1;

    // Scan for a line that contains exactly the delimiter (trimmed).
    let mut i = body_start;
    while i < bytes.len() {
        let line_end = find_newline_at_or_after(bytes, i).unwrap_or(bytes.len());
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

fn find_newline_at_or_after(bytes: &[u8], start: usize) -> Option<usize> {
    find_newline_after(bytes, start)
}

fn find_toplevel_brace_close(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    let max = bytes.len().min(start + 8192);
    let mut i = start;
    while i < max {
        let b = bytes[i];
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else {
            match b {
                b'"' => in_str = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
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
    fn fenced_ts_becomes_region() {
        let src = "# Title\n\n```ts\nexport const x = 1;\n```\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "typescript");
        assert_eq!(regions[0].origin, EmbeddedOrigin::MarkdownFence);
        assert!(regions[0].text.contains("export const x"));
    }

    #[test]
    fn unknown_fence_skipped() {
        let src = "```mermaid\ngraph TD\n```\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn yaml_frontmatter() {
        let src = "---\ntitle: Post\ntags: [a, b]\n---\n\n# Body\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "yaml");
        assert_eq!(regions[0].origin, EmbeddedOrigin::MarkdownFrontmatter);
        assert!(regions[0].text.contains("title: Post"));
    }

    #[test]
    fn toml_frontmatter() {
        let src = "+++\ntitle = \"Post\"\n+++\n\nbody\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "toml");
    }

    #[test]
    fn json_frontmatter_at_bof() {
        let src = "{\n  \"title\": \"Post\"\n}\n\nbody\n";
        let regions = detect_regions(src);
        assert!(
            regions
                .iter()
                .any(|r| r.language_id == "json" && r.origin == EmbeddedOrigin::MarkdownFrontmatter)
        );
    }

    #[test]
    fn frontmatter_and_fence_coexist() {
        let src = "---\ntitle: X\n---\n\n```rust\nfn main() {}\n```\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 2);
        assert!(regions.iter().any(|r| r.language_id == "yaml"));
        assert!(regions.iter().any(|r| r.language_id == "rust"));
    }

    #[test]
    fn fence_line_offset_is_body_start() {
        let src = "# h\n\ntext\n\n```rust\nfn f() {}\n```\n";
        let regions = detect_regions(src);
        let fence = regions.iter().find(|r| r.language_id == "rust").unwrap();
        // Body starts on line 5 (0-indexed).
        assert_eq!(fence.line_offset, 5);
    }

    #[test]
    fn empty_source_no_regions() {
        let regions = detect_regions("");
        assert!(regions.is_empty());
    }
}
