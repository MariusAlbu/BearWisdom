//! Rust doc-comment embedded regions.
//!
//! Finds runs of `///` outer doc comments and `//!` inner doc comments,
//! strips the leading prefix from each line, then re-parses the combined
//! Markdown-shaped text with the shared fenced-block parser. Each fence
//! whose info-string normalizes to a known language id becomes an
//! `EmbeddedRegion` with `origin = MarkdownFence` so spliced-in symbols
//! get flagged as snippet-origin and their unresolved refs don't pollute
//! project resolution stats.
//!
//! This is how Rust doc-tests (` /// ``` ... /// ``` `) surface into the
//! code graph:
//!
//! ```text
//! /// Compute the value.
//! ///
//! /// ```rust
//! /// let x = compute(1);
//! /// assert_eq!(x, 2);
//! /// ```
//! pub fn compute(n: u32) -> u32 { n + 1 }
//! ```

use crate::languages::markdown::fenced;
use crate::languages::markdown::info_string;
use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    for run in collect_doc_comment_runs(source) {
        // Strip the leading `///` or `//!` from each line (plus one
        // optional space) so what remains is Markdown-shaped.
        let stripped = run
            .lines
            .iter()
            .map(|(content, _line_no)| content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for fence in fenced::parse_fences(&stripped) {
            // Default doctest language is Rust when the info-string is
            // empty — this is how rustdoc treats bare ``` fences.
            let lang = if fence.info.is_empty() {
                "rust"
            } else {
                match info_string::normalize(&fence.info) {
                    Some(l) => l,
                    None => continue,
                }
            };
            // Map the fence's body line offset (relative to the stripped
            // text) back to an absolute line number in the source file.
            // `fence.body_line_offset` is the line index in the stripped
            // text where the body begins; the original source's line is
            // `run.lines[body_line_offset].1`.
            let abs_line = run
                .lines
                .get(fence.body_line_offset as usize)
                .map(|(_, l)| *l)
                .unwrap_or(run.first_line);
            regions.push(EmbeddedRegion {
                language_id: lang.to_string(),
                text: fence.body,
                line_offset: abs_line,
                col_offset: 0,
                origin: EmbeddedOrigin::MarkdownFence,
                holes: Vec::new(),
                strip_scope_prefix: None,
            });
        }
    }
    regions
}

/// A contiguous run of doc-comment lines, each with its original source
/// line number. Each line's content is the text AFTER the leading
/// `///` / `//!` prefix and one optional space.
struct DocRun {
    first_line: u32,
    lines: Vec<(String, u32)>,
}

fn collect_doc_comment_runs(source: &str) -> Vec<DocRun> {
    let mut runs: Vec<DocRun> = Vec::new();
    let mut current: Option<DocRun> = None;
    for (line_no, raw) in source.lines().enumerate() {
        let trimmed = raw.trim_start();
        let prefix = if trimmed.starts_with("///") {
            Some("///")
        } else if trimmed.starts_with("//!") {
            Some("//!")
        } else {
            None
        };
        match prefix {
            Some(p) => {
                let after = &trimmed[p.len()..];
                let content = after.strip_prefix(' ').unwrap_or(after);
                let ln = line_no as u32;
                if let Some(run) = current.as_mut() {
                    run.lines.push((content.to_string(), ln));
                } else {
                    current = Some(DocRun {
                        first_line: ln,
                        lines: vec![(content.to_string(), ln)],
                    });
                }
            }
            None => {
                if let Some(run) = current.take() {
                    runs.push(run);
                }
            }
        }
    }
    if let Some(run) = current {
        runs.push(run);
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_doctest_fence_emits_region() {
        let src = r#"
/// Compute the value.
///
/// ```rust
/// let x = 1 + 1;
/// assert_eq!(x, 2);
/// ```
pub fn compute() {}
"#;
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "rust");
        assert_eq!(regions[0].origin, EmbeddedOrigin::MarkdownFence);
        assert!(regions[0].text.contains("let x = 1 + 1;"));
    }

    #[test]
    fn bare_fence_defaults_to_rust() {
        let src = r#"
/// Example.
///
/// ```
/// let x = 1;
/// ```
pub fn f() {}
"#;
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "rust");
    }

    #[test]
    fn inner_doc_comment_also_collected() {
        let src = r#"
//! Crate-level docs.
//!
//! ```
//! let y = 42;
//! ```
"#;
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "rust");
    }

    #[test]
    fn non_doc_comments_ignored() {
        let src = "// plain\n// ```\n// let x = 1;\n// ```\npub fn f() {}\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn ts_fence_in_doc_comment() {
        let src = r#"
/// Interop example:
///
/// ```ts
/// const x: number = 1;
/// ```
pub fn f() {}
"#;
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "typescript");
    }

    #[test]
    fn line_offset_points_back_into_source() {
        let src = "\n\n/// doc\n/// \n/// ```rust\n/// let x = 1;\n/// ```\npub fn f() {}\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        // Body starts at the line AFTER ```rust — source line 5 (0-indexed).
        assert_eq!(regions[0].line_offset, 5);
    }
}
