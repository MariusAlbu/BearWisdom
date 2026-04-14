//! Python docstring and doc-fence embedded regions.
//!
//! Produces two kinds of regions out of module / class / function
//! docstrings:
//!
//!   * `>>> ` doctest lines (Python's standard doctest convention) —
//!     each run of `>>>` / `...` continuation lines becomes a single
//!     Python region with the expression text concatenated.
//!
//!   * Markdown-style fenced code blocks inside the docstring (common
//!     in NumPy/Google/Sphinx docstrings): ` ``` ... ``` ` — the body
//!     is dispatched to the language named by the info-string, same as
//!     the Markdown plugin.
//!
//! Both kinds use `origin = MarkdownFence` so spliced-in symbols are
//! flagged as snippet-origin and unresolved refs don't pollute
//! project-level resolution stats.

use crate::languages::markdown::fenced;
use crate::languages::markdown::info_string;
use crate::languages::string_dsl;
use crate::types::{EmbeddedOrigin, EmbeddedRegion};

pub fn detect_regions(source: &str) -> Vec<EmbeddedRegion> {
    let mut regions = Vec::new();
    for docstring in find_docstrings(source) {
        collect_doctests(&docstring, &mut regions);
        collect_fences(&docstring, &mut regions);
        // If the docstring body itself sniffs as a DSL (SQL / HTML /
        // JSON / CSS), emit a StringDsl region. Rare for actual
        // docstrings but common for triple-quoted string assignments
        // that `find_docstrings` also picks up.
        if let Some(lang_id) = string_dsl::sniff(&docstring.text) {
            regions.push(EmbeddedRegion {
                language_id: lang_id.to_string(),
                text: docstring.text.clone(),
                line_offset: docstring.line_offset,
                col_offset: 0,
                origin: EmbeddedOrigin::StringDsl,
                holes: Vec::new(),
                strip_scope_prefix: None,
            });
        }
    }
    regions
}

/// A recovered docstring: the text with the opening / closing triple
/// quotes removed, plus the 0-based source line where the docstring's
/// text begins (the line AFTER the opening quotes).
struct Docstring {
    text: String,
    line_offset: u32,
}

fn find_docstrings(source: &str) -> Vec<Docstring> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0usize;
    let mut line_no: u32 = 0;
    while i < bytes.len() {
        // Scan for triple-quote openings. Accept both """ and '''.
        if let Some((quote_char, open_start)) = find_triple_quote_start(bytes, i) {
            // Advance line count from i to open_start.
            line_no += bytes[i..open_start].iter().filter(|&&b| b == b'\n').count() as u32;
            let body_start = open_start + 3;
            // Find the matching closing triple-quote.
            let Some(body_end) = find_triple_quote_close(bytes, body_start, quote_char) else {
                break;
            };
            // Text line offset is the line where body_start sits.
            // If body starts with a newline, the docstring body's first
            // real line is body_start+1.
            let open_line_count = bytes[open_start..body_start]
                .iter()
                .filter(|&&b| b == b'\n')
                .count() as u32;
            let body_line = line_no + open_line_count;
            let text = std::str::from_utf8(&bytes[body_start..body_end])
                .unwrap_or("")
                .to_string();
            out.push(Docstring {
                text,
                line_offset: body_line,
            });
            // Advance past closing triple-quote. Update line_no up to
            // close end so subsequent searches are positioned right.
            line_no += bytes[open_start..body_end + 3]
                .iter()
                .filter(|&&b| b == b'\n')
                .count() as u32;
            i = body_end + 3;
        } else {
            break;
        }
    }
    out
}

fn find_triple_quote_start(bytes: &[u8], start: usize) -> Option<(u8, usize)> {
    let mut i = start;
    while i + 2 < bytes.len() {
        let b = bytes[i];
        if (b == b'"' || b == b'\'') && bytes[i + 1] == b && bytes[i + 2] == b {
            return Some((b, i));
        }
        i += 1;
    }
    None
}

fn find_triple_quote_close(bytes: &[u8], start: usize, quote: u8) -> Option<usize> {
    let mut i = start;
    while i + 2 < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            i += 2;
            continue;
        }
        if b == quote && bytes[i + 1] == quote && bytes[i + 2] == quote {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn collect_doctests(doc: &Docstring, out: &mut Vec<EmbeddedRegion>) {
    let mut current_expr = String::new();
    let mut current_line: Option<u32> = None;
    let lines: Vec<&str> = doc.text.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let stripped = line.trim_start();
        if let Some(rest) = stripped.strip_prefix(">>> ") {
            if !current_expr.is_empty() {
                out.push(make_doctest_region(
                    &current_expr,
                    doc.line_offset + current_line.unwrap_or(0),
                ));
                current_expr.clear();
            }
            current_expr.push_str(rest);
            current_expr.push('\n');
            current_line = Some(idx as u32);
        } else if stripped == ">>>" {
            if !current_expr.is_empty() {
                out.push(make_doctest_region(
                    &current_expr,
                    doc.line_offset + current_line.unwrap_or(0),
                ));
                current_expr.clear();
                current_line = None;
            }
        } else if let Some(rest) = stripped.strip_prefix("... ") {
            if current_line.is_some() {
                current_expr.push_str(rest);
                current_expr.push('\n');
            }
        } else if stripped == "..." {
            // Empty continuation — keep current expression open.
        } else if current_line.is_some() {
            // End of the doctest — the line is the expected output.
            out.push(make_doctest_region(
                &current_expr,
                doc.line_offset + current_line.unwrap_or(0),
            ));
            current_expr.clear();
            current_line = None;
        }
    }
    if !current_expr.is_empty() {
        out.push(make_doctest_region(
            &current_expr,
            doc.line_offset + current_line.unwrap_or(0),
        ));
    }
}

fn make_doctest_region(text: &str, line_offset: u32) -> EmbeddedRegion {
    EmbeddedRegion {
        language_id: "python".to_string(),
        text: text.to_string(),
        line_offset,
        col_offset: 0,
        origin: EmbeddedOrigin::MarkdownFence,
        holes: Vec::new(),
        strip_scope_prefix: None,
    }
}

fn collect_fences(doc: &Docstring, out: &mut Vec<EmbeddedRegion>) {
    for fence in fenced::parse_fences(&doc.text) {
        let lang = match info_string::normalize(&fence.info) {
            Some(l) => l,
            None if fence.info.is_empty() => "python",
            None => continue,
        };
        out.push(EmbeddedRegion {
            language_id: lang.to_string(),
            text: fence.body,
            line_offset: doc.line_offset + fence.body_line_offset,
            col_offset: 0,
            origin: EmbeddedOrigin::MarkdownFence,
            holes: Vec::new(),
            strip_scope_prefix: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_docstring_doctest_extracted() {
        let src = r#"
"""Compute things.

>>> compute(2)
4
"""

def compute(n):
    return n * 2
"#;
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "python"
            && r.origin == EmbeddedOrigin::MarkdownFence
            && r.text.contains("compute(2)")));
    }

    #[test]
    fn multi_line_doctest_with_continuation() {
        let src = r#"
"""
>>> x = [
...     1,
...     2,
... ]
>>> sum(x)
3
"""
"#;
        let regions = detect_regions(src);
        // At least two regions (one per >>> expression).
        assert!(regions.len() >= 2);
        assert!(regions.iter().any(|r| r.text.contains("sum(x)")));
    }

    #[test]
    fn fenced_block_inside_docstring() {
        let src = r#"
"""Usage.

```python
result = compute(3)
```
"""
"#;
        let regions = detect_regions(src);
        assert!(regions.iter().any(|r| r.language_id == "python"
            && r.text.contains("result = compute(3)")));
    }

    #[test]
    fn single_quote_triple_docstring_supported() {
        let src = "\n'''\n>>> do_thing()\n'''\n";
        let regions = detect_regions(src);
        assert_eq!(regions.len(), 1);
        assert!(regions[0].text.contains("do_thing()"));
    }

    #[test]
    fn no_docstring_no_regions() {
        let src = "def f():\n    return 1\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn non_doctest_prose_ignored() {
        let src = "\n\"\"\"\nThis module does things.\nIt is important.\n\"\"\"\n";
        let regions = detect_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn triple_quoted_sql_emits_string_dsl_region() {
        let src = "query = \"\"\"\nSELECT id, name FROM users WHERE active = 1\n\"\"\"\n";
        let regions = detect_regions(src);
        let sql = regions.iter().find(|r| r.language_id == "sql").expect("sql region");
        assert_eq!(sql.origin, EmbeddedOrigin::StringDsl);
    }
}
