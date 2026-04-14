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

/// Rust UI-framework macros whose body is HTML-like markup. Detected at
/// `macro_invocation` nodes whose macro path's final segment matches one
/// of these. The body of the enclosing token-tree becomes an `html` region
/// with `EmbeddedOrigin::TemplateExpr`.
const VIEW_MACRO_NAMES: &[&str] = &[
    "view",   // leptos::view! { ... }
    "html",   // yew::html! { ... }
    "rsx",    // dioxus::rsx! { ... }
    "rhtml",  // some crates
];

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = detect_doc_fence_regions(source);
    regions.extend(detect_view_macro_regions(source));
    regions
}

fn detect_doc_fence_regions(source: &str) -> Vec<EmbeddedRegion> {
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

/// Walk the Rust tree-sitter AST and emit an HTML-ish region for every
/// `view!` / `html!` / `rsx!` macro invocation. The body is the token-tree
/// with its outer delimiters stripped. Interpolations `{expr}` are left in
/// the text — the HTML sub-extractor tolerates stray braces in attribute
/// values well enough for component-name extraction, which is the main
/// signal we want to preserve.
fn detect_view_macro_regions(source: &str) -> Vec<EmbeddedRegion> {
    use tree_sitter::Parser;
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut regions = Vec::new();
    walk_for_view_macros(&tree.root_node(), source, &mut regions);
    regions
}

fn walk_for_view_macros(
    node: &tree_sitter::Node,
    source: &str,
    regions: &mut Vec<EmbeddedRegion>,
) {
    if node.kind() == "macro_invocation" {
        if let Some(region) = try_extract_view_macro(node, source) {
            regions.push(region);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_view_macros(&child, source, regions);
    }
}

fn try_extract_view_macro(
    node: &tree_sitter::Node,
    source: &str,
) -> Option<EmbeddedRegion> {
    let macro_node = node.child_by_field_name("macro")?;
    let macro_text = source.get(macro_node.start_byte()..macro_node.end_byte())?;
    // Final path segment — `leptos::view` → `view`.
    let last_seg = macro_text.rsplit("::").next().unwrap_or(macro_text);
    let name = last_seg.trim_end_matches('!').trim();
    if !VIEW_MACRO_NAMES.contains(&name) {
        return None;
    }
    // The token-tree child holds the `{...}`, `(...)`, or `[...]` body.
    let mut cursor = node.walk();
    let token_tree = node
        .children(&mut cursor)
        .find(|c| c.kind() == "token_tree")?;
    let body_text = source.get(token_tree.start_byte()..token_tree.end_byte())?;
    if body_text.len() < 2 {
        return None;
    }
    // Strip outer delimiters.
    let inner = &body_text[1..body_text.len() - 1];
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Body position skips the opening delimiter.
    let start_byte = token_tree.start_byte() + 1;
    let (line_offset, col_offset) = byte_to_line_col(source, start_byte);
    Some(EmbeddedRegion {
        language_id: "html".to_string(),
        text: inner.to_string(),
        line_offset,
        col_offset,
        origin: EmbeddedOrigin::TemplateExpr,
        holes: Vec::new(),
        strip_scope_prefix: None,
    })
}

fn byte_to_line_col(source: &str, byte: usize) -> (u32, u32) {
    let prefix = &source[..byte.min(source.len())];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
    let col = match prefix.rfind('\n') {
        Some(nl) => (byte - nl - 1) as u32,
        None => byte as u32,
    };
    (line, col)
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

    #[test]
    fn leptos_view_macro_emits_html_region() {
        let src = "fn app() { view! { <Button on_click=handle>Click</Button> } }";
        let regions = detect_regions(src);
        let view_region = regions.iter().find(|r| r.language_id == "html").expect("html region");
        assert_eq!(view_region.origin, EmbeddedOrigin::TemplateExpr);
        assert!(view_region.text.contains("<Button"));
        assert!(view_region.text.contains("Click"));
    }

    #[test]
    fn yew_html_macro_emits_region() {
        let src = "fn view() { html! { <div class=\"foo\"><p>{name}</p></div> } }";
        let regions = detect_regions(src);
        let r = regions.iter().find(|r| r.language_id == "html").expect("html region");
        assert!(r.text.contains("<div"));
        assert!(r.text.contains("<p"));
    }

    #[test]
    fn dioxus_rsx_macro_emits_region() {
        let src = "fn app() { rsx! { div { \"hi\" } } }";
        let regions = detect_regions(src);
        // rsx uses non-HTML syntax but we still emit a region — the HTML
        // sub-parser may find nothing, which is fine.
        assert_eq!(regions.iter().filter(|r| r.language_id == "html").count(), 1);
    }

    #[test]
    fn scoped_leptos_view_path_recognized() {
        let src = "fn a() { leptos::view! { <A/> } }";
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "html"));
    }

    #[test]
    fn non_view_macro_ignored() {
        let src = "fn main() { println!(\"hi\"); vec![1,2,3]; }";
        let regions = detect_regions(src);
        assert!(regions.iter().all(|r| r.language_id != "html"));
    }
}
