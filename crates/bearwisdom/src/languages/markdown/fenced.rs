//! Fenced code block parser (shared by Markdown, MDX, and doc-comment
//! hosts).
//!
//! Finds runs of ` ``` ` (or `~~~`) that open and close code fences in
//! Markdown-style text, preserves the info-string, and yields the body
//! with absolute byte + line offsets so the caller can splice the region
//! back into a host file with correct line attribution.
//!
//! This is NOT tree-sitter driven — Markdown fenced blocks are very
//! regular and a byte scanner is simpler, faster, and avoids pulling
//! tree-sitter-md through host plugins (Rust, Python) that just want to
//! parse doc-comment examples.
//!
//! # Examples
//!
//! ```text
//! ```ts
//! export const x = 1;
//! ```
//! ```
//!
//! …yields one `Fence { info: "ts", body: "export const x = 1;\n",
//! body_byte_offset: N, body_line_offset: L }`.

/// A single fenced code block.
#[derive(Debug, Clone)]
pub struct Fence {
    /// Raw info-string after the opening fence (trimmed). May contain
    /// attributes (`{.rust #anchor}`), notebook chunk options (`{r,
    /// echo=FALSE}`), or modifiers (`rust,no_run`). Use
    /// [`info_string::normalize`] to resolve to a canonical language id.
    pub info: String,
    /// The code-block body, EXCLUDING the opening and closing fence
    /// lines. Always terminated with a trailing newline if non-empty
    /// (preserved from the source).
    pub body: String,
    /// Absolute byte offset in the source where `body` begins.
    pub body_byte_offset: usize,
    /// 0-based line number in the source where `body` begins (the line
    /// AFTER the opening fence).
    pub body_line_offset: u32,
}

/// Parse all fenced code blocks out of `source`.
///
/// Recognizes both backtick (```` ``` ````) and tilde (`~~~`) fences.
/// The closing fence must be the same character as the opening fence
/// and at least as long. Unclosed fences are treated as running to
/// end-of-source.
pub fn parse_fences(source: &str) -> Vec<Fence> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let lines = line_starts(bytes);

    let mut i = 0usize;
    while i < lines.len() {
        let line_start = lines[i];
        let line_end = lines.get(i + 1).copied().unwrap_or(bytes.len());
        let line = &bytes[line_start..line_end];
        if let Some((ch, fence_len, info_start)) = detect_open_fence(line) {
            let info_end_rel = line[info_start..]
                .iter()
                .position(|&b| b == b'\n' || b == b'\r')
                .unwrap_or(line.len() - info_start);
            let info = std::str::from_utf8(&line[info_start..info_start + info_end_rel])
                .unwrap_or("")
                .trim()
                .to_string();

            let body_byte_offset = line_end;
            let body_line_offset = (i + 1) as u32;

            // Find closing fence.
            let mut close_line = i + 1;
            while close_line < lines.len() {
                let cs = lines[close_line];
                let ce = lines.get(close_line + 1).copied().unwrap_or(bytes.len());
                let l = &bytes[cs..ce];
                if is_close_fence(l, ch, fence_len) {
                    break;
                }
                close_line += 1;
            }
            let body_end = if close_line < lines.len() {
                lines[close_line]
            } else {
                bytes.len()
            };
            let body = std::str::from_utf8(&bytes[body_byte_offset..body_end])
                .unwrap_or("")
                .to_string();
            out.push(Fence {
                info,
                body,
                body_byte_offset,
                body_line_offset,
            });
            // Resume scanning AFTER the closing fence.
            i = close_line + 1;
            continue;
        }
        i += 1;
    }
    out
}

/// Return (fence_char, fence_len, info_string_start_index) if `line`
/// opens a fence. A fence opener is a run of 3+ backticks or 3+ tildes
/// optionally preceded by up to 3 spaces of indentation, followed by an
/// info-string (which may be empty).
fn detect_open_fence(line: &[u8]) -> Option<(u8, usize, usize)> {
    let mut i = 0;
    // Allow up to 3 leading spaces per CommonMark.
    while i < line.len() && i < 3 && line[i] == b' ' {
        i += 1;
    }
    if i >= line.len() {
        return None;
    }
    let ch = line[i];
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let run_start = i;
    while i < line.len() && line[i] == ch {
        i += 1;
    }
    let run_len = i - run_start;
    if run_len < 3 {
        return None;
    }
    // Backtick fences disallow additional backticks in the info string.
    if ch == b'`' {
        let rest = &line[i..];
        if rest.iter().any(|&b| b == b'`') {
            return None;
        }
    }
    Some((ch, run_len, i))
}

fn is_close_fence(line: &[u8], ch: u8, open_len: usize) -> bool {
    let mut i = 0;
    while i < line.len() && i < 3 && line[i] == b' ' {
        i += 1;
    }
    let run_start = i;
    while i < line.len() && line[i] == ch {
        i += 1;
    }
    let run_len = i - run_start;
    if run_len < open_len {
        return false;
    }
    // Only whitespace after the run.
    line[i..]
        .iter()
        .all(|&b| b == b' ' || b == b'\t' || b == b'\r' || b == b'\n')
}

fn line_starts(bytes: &[u8]) -> Vec<usize> {
    let mut out = Vec::with_capacity(bytes.len() / 40 + 1);
    out.push(0);
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            out.push(i + 1);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_fence() {
        let src = "text\n```ts\nexport const x = 1;\n```\nmore\n";
        let fences = parse_fences(src);
        assert_eq!(fences.len(), 1);
        assert_eq!(fences[0].info, "ts");
        assert_eq!(fences[0].body, "export const x = 1;\n");
        assert_eq!(fences[0].body_line_offset, 2);
    }

    #[test]
    fn parses_multiple_languages() {
        let src = "```rust\nfn f() {}\n```\n\n```python\nprint('hi')\n```\n";
        let fences = parse_fences(src);
        assert_eq!(fences.len(), 2);
        assert_eq!(fences[0].info, "rust");
        assert_eq!(fences[1].info, "python");
    }

    #[test]
    fn parses_tilde_fence() {
        let src = "~~~ts\nlet x = 1;\n~~~\n";
        let fences = parse_fences(src);
        assert_eq!(fences.len(), 1);
        assert_eq!(fences[0].body.trim(), "let x = 1;");
    }

    #[test]
    fn handles_unclosed_fence() {
        let src = "```ts\nexport const x = 1;\n";
        let fences = parse_fences(src);
        assert_eq!(fences.len(), 1);
        assert!(fences[0].body.contains("export const x"));
    }

    #[test]
    fn handles_empty_body() {
        let src = "```ts\n```\n";
        let fences = parse_fences(src);
        assert_eq!(fences.len(), 1);
        assert_eq!(fences[0].body, "");
    }

    #[test]
    fn indented_fence_up_to_three_spaces() {
        let src = "   ```ts\nconst x = 1;\n   ```\n";
        let fences = parse_fences(src);
        assert_eq!(fences.len(), 1);
    }

    #[test]
    fn fence_with_attributes() {
        let src = "```{.rust #myblock}\nfn f() {}\n```\n";
        let fences = parse_fences(src);
        assert_eq!(fences.len(), 1);
        assert_eq!(fences[0].info, "{.rust #myblock}");
    }

    #[test]
    fn body_line_offset_is_correct() {
        let src = "# heading\n\nsome text\n\n```rust\nfn f() {}\n```\n";
        let fences = parse_fences(src);
        assert_eq!(fences[0].body_line_offset, 5);
    }
}
