// =============================================================================
// languages/robot/extract.rs  —  Robot Framework extractor
//
// No tree-sitter grammar — uses a section-aware line scanner.
//
// What we extract
// ---------------
// SYMBOLS:
//   Test      — entries in `*** Test Cases ***` section
//   Function  — entries in `*** Keywords ***` section
//   Variable  — entries in `*** Variables ***` section
//
// REFERENCES:
//   Imports   — `Library    <name>` / `Resource    <path>` in `*** Settings ***`
//   Calls     — keyword invocations inside test cases and keyword bodies
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use crate::types::ExtractionResult;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Section {
    None,
    Settings,
    Variables,
    TestCases,
    Keywords,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let lines: Vec<&str> = source.lines().collect();
    let mut section = Section::None;
    let mut current_item: Option<usize> = None; // index into symbols of current kw/tc

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Section header detection
        if trimmed.starts_with("***") && trimmed.ends_with("***") {
            section = detect_section(trimmed);
            current_item = None;
            i += 1;
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        match section {
            Section::Settings => {
                extract_settings_line(trimmed, i as u32, &symbols, &mut refs);
            }
            Section::Variables => {
                if let Some(var_name) = extract_variable_name(trimmed) {
                    symbols.push(make_symbol(
                        var_name.clone(), var_name, SymbolKind::Variable, i as u32, None,
                    ));
                    current_item = Some(symbols.len() - 1);
                }
            }
            Section::TestCases => {
                // A test case starts at column 0 (no leading whitespace)
                if !line.starts_with(' ') && !line.starts_with('\t') {
                    let name = trimmed.to_string();
                    symbols.push(make_symbol(
                        name.clone(), name, SymbolKind::Test, i as u32, None,
                    ));
                    current_item = Some(symbols.len() - 1);
                } else if let Some(idx) = current_item {
                    // Body line — keyword invocation
                    extract_keyword_invocation(trimmed, i as u32, idx, &mut refs);
                }
            }
            Section::Keywords => {
                if !line.starts_with(' ') && !line.starts_with('\t') {
                    let name = trimmed.to_string();
                    symbols.push(make_symbol(
                        name.clone(), name, SymbolKind::Function, i as u32, None,
                    ));
                    current_item = Some(symbols.len() - 1);
                } else if let Some(idx) = current_item {
                    // Body line — keyword invocation (or [Setting])
                    if !trimmed.starts_with('[') {
                        extract_keyword_invocation(trimmed, i as u32, idx, &mut refs);
                    }
                }
            }
            Section::None => {}
        }

        i += 1;
    }

    ExtractionResult::new(symbols, refs, false)
}

// ---------------------------------------------------------------------------
// Section detection
// ---------------------------------------------------------------------------

fn detect_section(trimmed: &str) -> Section {
    let lower = trimmed.to_lowercase();
    if lower.contains("test case") || lower.contains("test cases") {
        Section::TestCases
    } else if lower.contains("keyword") {
        Section::Keywords
    } else if lower.contains("setting") {
        Section::Settings
    } else if lower.contains("variable") {
        Section::Variables
    } else {
        Section::None
    }
}

// ---------------------------------------------------------------------------
// Settings section: Library / Resource imports
// ---------------------------------------------------------------------------

fn extract_settings_line(
    trimmed: &str,
    lineno: u32,
    symbols: &[ExtractedSymbol],
    refs: &mut Vec<ExtractedRef>,
) {
    let source_idx = symbols.len().saturating_sub(1);
    // Split on 2+ spaces or tabs (Robot Framework cell separator)
    let cells: Vec<&str> = split_cells(trimmed);
    if cells.is_empty() {
        return;
    }
    match cells[0].trim() {
        "Library" | "Resource" | "Variables" => {
            if let Some(target) = cells.get(1) {
                let t = target.trim().to_string();
                if !t.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: t.clone(),
                        kind: EdgeKind::Imports,
                        line: lineno,
                        module: Some(t),
                        chain: None,
                    });
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Variable name extraction: `${VAR}    value` → `VAR`
// ---------------------------------------------------------------------------

fn extract_variable_name(trimmed: &str) -> Option<String> {
    // Robot variable syntax: ${NAME}, @{NAME}, &{NAME}
    let start = trimmed.find(|c| c == '$' || c == '@' || c == '&')?;
    let rest = &trimmed[start + 1..];
    if !rest.starts_with('{') {
        return None;
    }
    let end = rest.find('}')?;
    let name = &rest[1..end];
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

// ---------------------------------------------------------------------------
// Keyword invocations: first cell is the keyword name
// ---------------------------------------------------------------------------

fn extract_keyword_invocation(
    trimmed: &str,
    lineno: u32,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let cells = split_cells(trimmed);
    if cells.is_empty() {
        return;
    }

    // First cell: keyword name (skip [Settings], RETURN, FOR, IF, etc.)
    let kw = cells[0].trim();
    if kw.starts_with('[') || kw.starts_with('#') || kw.is_empty() {
        return;
    }
    // Skip Robot framework control structures
    if matches!(kw, "FOR" | "END" | "IF" | "ELSE" | "ELSE IF" | "WHILE" | "TRY" | "EXCEPT" | "FINALLY" | "RETURN" | "BREAK" | "CONTINUE") {
        return;
    }
    // Handle `${var} =    Keyword` assignment pattern
    let keyword_name = if kw.contains('{') {
        // `${var} =` or `${var}=` assignment — keyword is the second cell
        cells.get(1).map(|s| s.trim()).unwrap_or("")
    } else {
        kw
    };
    if keyword_name.is_empty() || keyword_name.starts_with('[') {
        return;
    }

    // Split `Library.Keyword Name` into module + keyword.
    // The first `.` separates the library name from the keyword name.
    // Only treat as qualified if the prefix looks like a library name
    // (no spaces, no variable sigils).
    let (module, target_name) = if let Some(dot) = keyword_name.find('.') {
        let prefix = &keyword_name[..dot];
        let suffix = keyword_name[dot + 1..].trim();
        if !prefix.is_empty()
            && !prefix.contains(' ')
            && !prefix.contains('{')
            && !suffix.is_empty()
        {
            (Some(prefix.to_string()), suffix.to_string())
        } else {
            (None, keyword_name.to_string())
        }
    } else {
        (None, keyword_name.to_string())
    };

    refs.push(ExtractedRef {
        source_symbol_index: source_idx,
        target_name,
        kind: EdgeKind::Calls,
        line: lineno,
        module,
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Split a Robot Framework line into cells (separated by 2+ spaces or tab).
fn split_cells(line: &str) -> Vec<&str> {
    // Split on `  ` (2+ spaces) or `\t`
    let mut cells = Vec::new();
    let mut start = 0;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\t' {
            cells.push(&line[start..i]);
            start = i + 1;
            i += 1;
        } else if bytes[i] == b' ' && i + 1 < bytes.len() && bytes[i + 1] == b' ' {
            cells.push(&line[start..i]);
            // Skip all consecutive spaces
            while i < bytes.len() && bytes[i] == b' ' { i += 1; }
            start = i;
        } else {
            i += 1;
        }
    }
    if start < bytes.len() {
        cells.push(&line[start..]);
    }
    // Filter empty cells
    cells.into_iter().filter(|c| !c.trim().is_empty()).collect()
}

fn make_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    line: u32,
    signature: Option<String>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}
