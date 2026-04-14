//! Jupyter magic-line stripping.
//!
//! Lines starting with `!` (shell), `%` (line magic), or `%%` (cell
//! magic) are valid inside a Jupyter code cell but are invalid
//! syntax for the target language (Python, R, etc.). They're
//! blanked to preserve line numbers while keeping the cell body
//! parseable.
//!
//! Cell magics (`%%bash`, `%%javascript`, `%%html`) actually change
//! the cell's effective language. A principled future expansion
//! would dispatch such cells to a different language plugin; for
//! now, we blank the magic line and let the parser see the rest as
//! though it were still the kernel's language.

/// Strip Jupyter magic lines from `src`, replacing each with a
/// blank line so line numbers stay stable.
pub fn strip_magics(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in split_preserving_newlines(src) {
        let trimmed = line.trim_start_matches([' ', '\t']);
        if trimmed.starts_with('!') || trimmed.starts_with('%') {
            if line.ends_with('\n') {
                out.push('\n');
            }
            continue;
        }
        out.push_str(line);
    }
    out
}

fn split_preserving_newlines(src: &str) -> impl Iterator<Item = &str> {
    let mut start = 0;
    let bytes = src.as_bytes();
    let mut parts: Vec<&str> = Vec::new();
    for i in 0..bytes.len() {
        if bytes[i] == b'\n' {
            parts.push(&src[start..=i]);
            start = i + 1;
        }
    }
    if start < bytes.len() {
        parts.push(&src[start..]);
    }
    parts.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_bang_line_stripped() {
        let src = "!pip install numpy\nimport numpy as np\n";
        let out = strip_magics(src);
        assert_eq!(out, "\nimport numpy as np\n");
    }

    #[test]
    fn line_magic_stripped() {
        let src = "%timeit some_func()\nresult = compute()\n";
        let out = strip_magics(src);
        assert_eq!(out, "\nresult = compute()\n");
    }

    #[test]
    fn cell_magic_stripped() {
        let src = "%%bash\nls -la\necho hi\n";
        let out = strip_magics(src);
        assert_eq!(out, "\nls -la\necho hi\n");
    }

    #[test]
    fn leading_whitespace_before_magic_still_stripped() {
        let src = "    %%bash\nls\n";
        let out = strip_magics(src);
        assert_eq!(out, "\nls\n");
    }

    #[test]
    fn non_magic_line_preserved() {
        let src = "x = 1\ny = 2\n";
        let out = strip_magics(src);
        assert_eq!(out, "x = 1\ny = 2\n");
    }

    #[test]
    fn modulo_operator_not_confused_with_magic() {
        // `%` appearing mid-line is an operator, not a magic.
        let src = "result = 10 % 3\n";
        let out = strip_magics(src);
        assert_eq!(out, "result = 10 % 3\n");
    }
}
