//! Jupyter cell scanner.
//!
//! Parses the notebook JSON, extracts each cell's `cell_type` and
//! `source` content, and locates the byte position of the `source`
//! value in the original JSON text so embedded regions can carry a
//! correct `line_offset` relative to the `.ipynb` file.
//!
//! The structural parse uses `serde_json`. Byte-offset location is
//! a separate lightweight scan: for each cell we step through the
//! raw source text looking for the cell's `"source"` field and
//! record the line number where its value begins.

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Cell {
    pub cell_type: CellKind,
    pub body: String,
    /// 0-based line in the `.ipynb` file where the cell's `source`
    /// content effectively starts. For an array-form source this is
    /// the line of the first element string; for a string-form
    /// source it's the line of the opening quote.
    pub body_line_offset: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    Code,
    Markdown,
    Raw,
    Other,
}

/// Parsed notebook summary.
pub struct Notebook {
    /// Kernel language from `metadata.kernelspec.language`, or
    /// `"python"` if absent (Jupyter's de facto default).
    pub kernel_language: String,
    pub cells: Vec<Cell>,
}

/// Parse the notebook text into a structured `Notebook`. Returns
/// `None` on malformed JSON or missing `cells` array.
pub fn parse_notebook(source: &str) -> Option<Notebook> {
    let root: Value = serde_json::from_str(source).ok()?;
    let kernel_language = root
        .get("metadata")
        .and_then(|m| m.get("kernelspec"))
        .and_then(|k| k.get("language"))
        .and_then(|l| l.as_str())
        .unwrap_or("python")
        .to_ascii_lowercase();
    let cells_value = root.get("cells")?.as_array()?;

    let cell_kinds: Vec<(CellKind, String)> = cells_value
        .iter()
        .map(|cell| {
            let kind = match cell.get("cell_type").and_then(|v| v.as_str()) {
                Some("code") => CellKind::Code,
                Some("markdown") => CellKind::Markdown,
                Some("raw") => CellKind::Raw,
                _ => CellKind::Other,
            };
            let body = cell
                .get("source")
                .map(concat_source)
                .unwrap_or_default();
            (kind, body)
        })
        .collect();

    // Second pass: for each cell, find the byte position of its
    // `"source"` field in the raw JSON text and convert to a line
    // number. Cells are delimited by their `cell_type` markers
    // encountered in order — naive but sufficient for well-formed
    // notebooks.
    let source_positions = locate_cell_source_positions(source, cells_value.len());

    let cells = cell_kinds
        .into_iter()
        .zip(source_positions.into_iter())
        .map(|((cell_type, body), body_line_offset)| Cell {
            cell_type,
            body,
            body_line_offset,
        })
        .collect();

    Some(Notebook {
        kernel_language,
        cells,
    })
}

/// Join a cell's `source` value — which may be a string or an array
/// of strings — into a single contiguous cell body.
fn concat_source(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(arr) => {
            let mut out = String::new();
            for item in arr {
                if let Some(s) = item.as_str() {
                    out.push_str(s);
                }
            }
            out
        }
        _ => String::new(),
    }
}

/// For each cell, locate the line number where its `source` value
/// begins in the raw JSON text. Returns one entry per cell in
/// document order.
///
/// Strategy: walk the JSON brace-depth-aware, find each object that
/// sits directly inside the top-level `cells` array, then inside
/// that object find the `"source"` key and record the line of the
/// value start.
fn locate_cell_source_positions(source: &str, expected_count: usize) -> Vec<u32> {
    let bytes = source.as_bytes();
    // Find the `"cells"` array start.
    let Some(cells_start) = find_key_value_start(bytes, b"cells") else {
        return vec![0; expected_count];
    };
    // `cells_start` points at `[` of the cells array.
    if bytes.get(cells_start).copied() != Some(b'[') {
        return vec![0; expected_count];
    }

    let mut positions = Vec::with_capacity(expected_count);
    let mut i = cells_start + 1;
    let mut depth: i32 = 1;
    let mut in_str = false;
    let mut escape = false;
    while i < bytes.len() && depth > 0 {
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
                b'[' | b'{' => {
                    if depth == 1 && b == b'{' {
                        // Start of a cell object. Scan inside until
                        // its matching `}` for the `"source"` key.
                        let cell_end = find_matching_brace(bytes, i);
                        let source_line = find_source_line_in_cell(bytes, i, cell_end);
                        positions.push(source_line);
                        i = cell_end;
                        continue;
                    }
                    depth += 1;
                }
                b']' | b'}' => depth -= 1,
                _ => {}
            }
        }
        i += 1;
    }

    while positions.len() < expected_count {
        positions.push(0);
    }
    positions.truncate(expected_count);
    positions
}

/// Scan for a top-level `"key":` and return the byte index of its
/// value start (whitespace-skipped).
fn find_key_value_start(bytes: &[u8], key: &[u8]) -> Option<usize> {
    let mut pattern = Vec::with_capacity(key.len() + 2);
    pattern.push(b'"');
    pattern.extend_from_slice(key);
    pattern.push(b'"');
    let mut i = 0;
    while i + pattern.len() <= bytes.len() {
        if &bytes[i..i + pattern.len()] == &pattern[..] {
            let mut j = i + pattern.len();
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n') {
                j += 1;
            }
            if bytes.get(j).copied() == Some(b':') {
                j += 1;
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n')
                {
                    j += 1;
                }
                return Some(j);
            }
        }
        i += 1;
    }
    None
}

/// Given the index of an opening `{`, find the matching `}`. Honors
/// string-literal quoting with escape handling.
fn find_matching_brace(bytes: &[u8], start: usize) -> usize {
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut escape = false;
    let mut i = start;
    while i < bytes.len() {
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
                        return i;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    bytes.len()
}

/// Within a cell object spanning `[start..=end]`, find the line
/// number where the `source` value begins. Returns 0 if not found.
fn find_source_line_in_cell(bytes: &[u8], start: usize, end: usize) -> u32 {
    let slice = &bytes[start..=end.min(bytes.len() - 1)];
    let Some(rel_value) = find_key_value_start(slice, b"source") else {
        return 0;
    };
    let abs_value = start + rel_value;
    // Move past the opening `[` or `"` to reach the first content line.
    let mut p = abs_value;
    while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t' || bytes[p] == b'\n') {
        p += 1;
    }
    if bytes.get(p).copied() == Some(b'[') {
        // Array form — advance to first non-whitespace byte after `[`.
        p += 1;
        while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t' || bytes[p] == b'\n') {
            p += 1;
        }
    }
    line_at(bytes, p)
}

fn line_at(bytes: &[u8], pos: usize) -> u32 {
    let mut line: u32 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if i >= pos {
            break;
        }
        if b == b'\n' {
            line += 1;
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_python_notebook_with_two_cells() {
        let src = r##"{
 "cells": [
  {"cell_type": "code", "source": ["x = 1\n", "y = 2\n"], "metadata": {}},
  {"cell_type": "markdown", "source": "# Title\n", "metadata": {}}
 ],
 "metadata": {"kernelspec": {"language": "python"}},
 "nbformat": 4
}
"##;
        let nb = parse_notebook(src).unwrap();
        assert_eq!(nb.kernel_language, "python");
        assert_eq!(nb.cells.len(), 2);
        assert_eq!(nb.cells[0].cell_type, CellKind::Code);
        assert_eq!(nb.cells[0].body, "x = 1\ny = 2\n");
        assert_eq!(nb.cells[1].cell_type, CellKind::Markdown);
    }

    #[test]
    fn kernel_language_defaults_to_python_when_absent() {
        let src = r##"{"cells": [], "metadata": {}, "nbformat": 4}"##;
        let nb = parse_notebook(src).unwrap();
        assert_eq!(nb.kernel_language, "python");
    }

    #[test]
    fn string_source_field_accepted() {
        let src = r##"{
 "cells": [{"cell_type": "code", "source": "x = 1\ny = 2\n", "metadata": {}}],
 "metadata": {"kernelspec": {"language": "python"}}
}"##;
        let nb = parse_notebook(src).unwrap();
        assert_eq!(nb.cells[0].body, "x = 1\ny = 2\n");
    }

    #[test]
    fn source_line_offset_points_to_content_line() {
        // Cell object starts on line 2, "source" value on line 3.
        let src = r##"{
 "cells": [
  {
   "cell_type": "code",
   "source": [
    "x = 1\n"
   ],
   "metadata": {}
  }
 ],
 "metadata": {"kernelspec": {"language": "python"}}
}"##;
        let nb = parse_notebook(src).unwrap();
        // The first content line is line 5 (0-indexed) — the `"x = 1\n"` line.
        assert_eq!(nb.cells[0].body_line_offset, 5);
    }

    #[test]
    fn malformed_json_returns_none() {
        assert!(parse_notebook("not json").is_none());
    }

    #[test]
    fn r_kernel_language_lowercased() {
        let src = r##"{"cells": [], "metadata": {"kernelspec": {"language": "R"}}}"##;
        let nb = parse_notebook(src).unwrap();
        assert_eq!(nb.kernel_language, "r");
    }
}
