//! Embedded-region detection for RMarkdown / Quarto.
//!
//! Every fenced block whose info-string normalizes to a known
//! language id becomes one `NotebookCell` region. The Markdown
//! plugin's `info_string::normalize` already handles `{r}`,
//! `{python, echo=FALSE}`, and the Pandoc `{.rust}` attribute form,
//! so no chunk-specific parser is needed.
//!
//! Frontmatter (YAML `---` or TOML `+++`) is also emitted the same
//! way as Markdown, so generic YAML/TOML extractors can process it.

use super::super::markdown::{fenced, info_string};
use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();

    let trimmed_start = skip_leading_whitespace(source);
    if let Some(fm) = detect_frontmatter(source, trimmed_start) {
        regions.push(fm);
    }

    for fence in fenced::parse_fences(source) {
        let Some(lang) = info_string::normalize(&fence.info) else {
            continue;
        };
        if matches!(lang, "json" | "xml") {
            continue;
        }
        regions.push(EmbeddedRegion {
            language_id: lang.to_string(),
            text: fence.body,
            line_offset: fence.body_line_offset,
            col_offset: 0,
            origin: EmbeddedOrigin::NotebookCell,
            holes: Vec::new(),
            strip_scope_prefix: None,
        });
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
    let open_end = bytes[start + delim_bytes.len()..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|p| start + delim_bytes.len() + p)?;
    let body_start = open_end + 1;

    let mut i = body_start;
    while i < bytes.len() {
        let line_end = bytes[i..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| i + p)
            .unwrap_or(bytes.len());
        let line = source.get(i..line_end)?;
        if line.trim() == delim {
            let body = source.get(body_start..i)?.to_string();
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
    fn r_chunk_becomes_region() {
        let src = "# Title\n\n```{r}\nlibrary(dplyr)\n```\n";
        let regions = detect_regions(src);
        let r = regions.iter().find(|r| r.language_id == "r").unwrap();
        assert_eq!(r.origin, EmbeddedOrigin::NotebookCell);
        assert!(r.text.contains("library(dplyr)"));
    }

    #[test]
    fn python_chunk_becomes_region() {
        let src = "```{python}\nimport pandas\n```\n";
        let regions = detect_regions(src);
        assert!(regions
            .iter()
            .any(|r| r.language_id == "python" && r.origin == EmbeddedOrigin::NotebookCell));
    }

    #[test]
    fn chunk_options_accepted() {
        let src = "```{r cars, echo=FALSE}\nsummary(cars)\n```\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "r"));
    }

    #[test]
    fn yaml_frontmatter_extracted() {
        let src = "---\ntitle: Rpt\nauthor: Ann\n---\n\n# Body\n";
        let regions = detect_regions(src);
        assert!(regions
            .iter()
            .any(|r| r.language_id == "yaml" && r.origin == EmbeddedOrigin::MarkdownFrontmatter));
    }

    #[test]
    fn unknown_chunk_language_skipped() {
        let src = "```{mermaid}\ngraph TD\n```\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn bash_chunk_becomes_region() {
        let src = "```{bash}\nls -la\n```\n";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "bash"));
    }
}
