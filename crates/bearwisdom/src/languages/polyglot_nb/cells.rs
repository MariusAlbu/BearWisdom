//! Shared cell parser for Polyglot Notebook (`.dib`) files.
//!
//! Splits source on `#!<lang>` markers at the start of a line. Each
//! cell captures its kernel id, body text, byte offset, and line
//! offset (the line AFTER the marker). The first run of lines
//! before any `#!` marker is ignored — .dib files conventionally
//! open with a kernel declaration, but tolerating a lead-in is cheap.

/// One notebook cell.
#[derive(Debug, Clone)]
pub struct Cell {
    /// Raw kernel identifier from the `#!<id>` marker (lowercased,
    /// whitespace-trimmed). Examples: `csharp`, `fsharp`, `pwsh`,
    /// `javascript`, `sql`, `kql`, `markdown`, `value`, `mermaid`.
    pub kernel: String,
    /// Cell body text (everything between this marker and the next,
    /// or EOF). Trailing newline preserved if present in source.
    pub body: String,
    /// Absolute byte offset in the source where `body` begins.
    pub body_byte_offset: usize,
    /// 0-based line number in the source where `body` begins (the
    /// line AFTER the `#!<lang>` marker line).
    pub body_line_offset: u32,
}

/// Parse all cells out of a `.dib` source.
pub fn parse_cells(source: &str) -> Vec<Cell> {
    let bytes = source.as_bytes();
    let mut line_starts: Vec<usize> = vec![0];
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }

    let mut markers: Vec<(usize, String)> = Vec::new();
    for (idx, &ls) in line_starts.iter().enumerate() {
        let le = line_starts.get(idx + 1).copied().unwrap_or(bytes.len());
        if let Some(kernel) = parse_marker(&bytes[ls..le]) {
            markers.push((idx, kernel));
        }
    }

    let mut cells = Vec::with_capacity(markers.len());
    for (n, (marker_line, kernel)) in markers.iter().enumerate() {
        let body_line = marker_line + 1;
        let body_byte = line_starts.get(body_line).copied().unwrap_or(bytes.len());
        let end_byte = if n + 1 < markers.len() {
            line_starts[markers[n + 1].0]
        } else {
            bytes.len()
        };
        let body = std::str::from_utf8(&bytes[body_byte..end_byte])
            .unwrap_or("")
            .to_string();
        cells.push(Cell {
            kernel: kernel.clone(),
            body,
            body_byte_offset: body_byte,
            body_line_offset: body_line as u32,
        });
    }
    cells
}

/// If `line` is `#!<ident>[ whitespace ]*\n`, return the lowercased
/// ident. Otherwise return None.
fn parse_marker(line: &[u8]) -> Option<String> {
    if line.len() < 3 || line[0] != b'#' || line[1] != b'!' {
        return None;
    }
    let mut i = 2;
    let start = i;
    while i < line.len() && (line[i].is_ascii_alphanumeric() || line[i] == b'-' || line[i] == b'_')
    {
        i += 1;
    }
    if i == start {
        return None;
    }
    // Remainder must be whitespace (ignore kernel options after the
    // ident — e.g. `#!csharp --display-name "Foo"`).
    let ident = std::str::from_utf8(&line[start..i]).ok()?.to_ascii_lowercase();
    Some(ident)
}

/// Map a `.dib` kernel id to the BearWisdom language id its code
/// should dispatch to. Returns `None` for non-code kernels
/// (`value`, `mermaid`) and for unknown kernels.
pub fn kernel_to_language_id(kernel: &str) -> Option<&'static str> {
    match kernel {
        "csharp" | "cs" | "c#" => Some("csharp"),
        "fsharp" | "fs" | "f#" => Some("fsharp"),
        "pwsh" | "powershell" | "ps1" => Some("powershell"),
        "javascript" | "js" => Some("javascript"),
        "typescript" | "ts" => Some("typescript"),
        "sql" | "kql" | "mssql" | "postgresql" => Some("sql"),
        "html" => Some("html"),
        "markdown" | "md" => Some("markdown"),
        "python" | "py" => Some("python"),
        // Non-code / display kernels — skip.
        "value" | "mermaid" | "share" => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_simple_two_cell_notebook() {
        let src = "#!csharp\nvar x = 1;\n\n#!fsharp\nlet y = 2\n";
        let cells = parse_cells(src);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].kernel, "csharp");
        assert!(cells[0].body.contains("var x = 1;"));
        assert_eq!(cells[1].kernel, "fsharp");
        assert!(cells[1].body.contains("let y = 2"));
    }

    #[test]
    fn body_line_offset_is_line_after_marker() {
        let src = "#!csharp\nline0\nline1\n";
        let cells = parse_cells(src);
        assert_eq!(cells[0].body_line_offset, 1);
    }

    #[test]
    fn marker_options_after_ident_ignored() {
        let src = "#!csharp --name foo\nvar x = 1;\n";
        let cells = parse_cells(src);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].kernel, "csharp");
    }

    #[test]
    fn kernel_aliases_mapped() {
        assert_eq!(kernel_to_language_id("csharp"), Some("csharp"));
        assert_eq!(kernel_to_language_id("pwsh"), Some("powershell"));
        assert_eq!(kernel_to_language_id("powershell"), Some("powershell"));
        assert_eq!(kernel_to_language_id("kql"), Some("sql"));
        assert_eq!(kernel_to_language_id("mermaid"), None);
        assert_eq!(kernel_to_language_id("unknown"), None);
    }

    #[test]
    fn content_before_first_marker_is_discarded() {
        let src = "prologue\n#!csharp\nvar x = 1;\n";
        let cells = parse_cells(src);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].kernel, "csharp");
    }

    #[test]
    fn empty_cell_ok() {
        let src = "#!csharp\n#!fsharp\nlet y = 2\n";
        let cells = parse_cells(src);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].body, "");
    }

    #[test]
    fn empty_source_no_cells() {
        assert!(parse_cells("").is_empty());
    }
}
