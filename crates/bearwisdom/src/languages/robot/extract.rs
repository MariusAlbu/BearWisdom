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
    // Suite-level `Test Template    <Keyword>` from `*** Settings ***`.
    // Applies to EVERY test in the file; per-test `[Template]` can
    // override (including `[Template]    NONE` to disable for one test).
    let mut suite_template_active: bool = false;
    // Tracks `[Template]    <Keyword>` for the active test or keyword. When
    // set, every subsequent body row is positional ARG data for the
    // template, not a keyword invocation — the first cell must NOT be
    // emitted as a Calls ref. Reset on each new test/keyword header to
    // the suite-level default.
    let mut template_active: bool = false;

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Section header detection
        if trimmed.starts_with("***") && trimmed.ends_with("***") {
            section = detect_section(trimmed);
            current_item = None;
            template_active = false;
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
                // Detect suite-level `Test Template    <Keyword>` so the
                // per-test default is "template active" unless a test
                // explicitly resets via `[Template]    NONE`.
                if let Some(rest) = strip_setting_keyword(trimmed, "Test Template") {
                    suite_template_active =
                        !rest.is_empty() && !rest.eq_ignore_ascii_case("NONE");
                }
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
                    template_active = suite_template_active;
                } else if let Some(idx) = current_item {
                    handle_body_line(
                        trimmed,
                        i as u32,
                        idx,
                        &mut refs,
                        &mut template_active,
                    );
                }
            }
            Section::Keywords => {
                if !line.starts_with(' ') && !line.starts_with('\t') {
                    let name = trimmed.to_string();
                    symbols.push(make_symbol(
                        name.clone(), name, SymbolKind::Function, i as u32, None,
                    ));
                    current_item = Some(symbols.len() - 1);
                    template_active = suite_template_active;
                } else if let Some(idx) = current_item {
                    handle_body_line(
                        trimmed,
                        i as u32,
                        idx,
                        &mut refs,
                        &mut template_active,
                    );
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
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
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
// Body lines inside Test Cases / Keywords sections.
//
// Two intertwined concerns the previous code missed:
//
//   1. `[Template]    <Keyword>` — once set on a test (or keyword), every
//      subsequent body row is a row of POSITIONAL ARGUMENTS to the
//      template. The first cell is data, NOT a keyword call. Without
//      tracking template state we leak hundreds of false-positive Calls
//      refs to literal arg values like `1`, `abcdefg`, `Hello, world!`.
//
//   2. `[Setup]    <Keyword>` and `[Teardown]    <Keyword>` — the second
//      cell IS a real keyword call. Other `[Setting]` lines (`[Tags]`,
//      `[Documentation]`, `[Arguments]`, `[Return]`, `[Timeout]`) carry
//      data only and must not produce Calls refs.
// ---------------------------------------------------------------------------

fn handle_body_line(
    trimmed: &str,
    lineno: u32,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
    template_active: &mut bool,
) {
    if trimmed.starts_with('[') {
        // Bracketed setting line. Parse it once and route by kind.
        let cells = split_cells(trimmed);
        let setting = cells.first().map(|s| s.trim()).unwrap_or("");
        match setting {
            "[Template]" => {
                // Any non-empty 2nd cell turns the template on. An empty
                // (or `NONE`) value disables templating for this test.
                let arg = cells.get(1).map(|s| s.trim()).unwrap_or("");
                *template_active = !arg.is_empty() && !arg.eq_ignore_ascii_case("NONE");
            }
            "[Setup]" | "[Teardown]" => {
                if let Some(kw) = cells.get(1).map(|s| s.trim()) {
                    if !kw.is_empty() && !kw.eq_ignore_ascii_case("NONE") {
                        emit_keyword_call(kw, lineno, source_idx, refs);
                    }
                }
            }
            _ => {
                // [Tags], [Documentation], [Arguments], [Return],
                // [Timeout], etc. — data only.
            }
        }
        return;
    }
    if *template_active {
        // Template active — body line is data, not a call.
        return;
    }
    extract_keyword_invocation(trimmed, lineno, source_idx, refs);
}

/// Emit a single Calls ref for `keyword_name`, applying the same
/// `Library.Keyword` splitting that `extract_keyword_invocation` uses.
/// Shared between the implicit-Setup/Teardown path and the regular
/// body-line path.
fn emit_keyword_call(
    keyword_name: &str,
    lineno: u32,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
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
        byte_offset: 0,
        namespace_segments: Vec::new(),
    });
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
    // Skip Robot framework control structures and non-call markers:
    //   `...`  — line continuation (appends args to previous line)
    //   `\END` — escaped END terminator used in older FOR/IF fixtures
    //   `VAR`  — Robot 6+ inline variable assignment, not a keyword call
    if matches!(
        kw,
        "FOR" | "END" | "IF" | "ELSE" | "ELSE IF" | "WHILE" | "TRY" | "EXCEPT" | "FINALLY"
            | "RETURN" | "BREAK" | "CONTINUE" | "..." | "\\END" | "VAR"
    ) {
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
        byte_offset: 0,
            namespace_segments: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Split a Robot Framework line into cells (separated by 2+ spaces or tab).
/// If `line` begins with the case-sensitive setting `keyword` followed by
/// at least two spaces (Robot's cell separator), return the remainder.
/// Otherwise return `None`. Used to detect suite-level settings like
/// `Test Template    Should Be Equal` from `*** Settings ***`.
fn strip_setting_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    if !line.starts_with(keyword) {
        return None;
    }
    let after = &line[keyword.len()..];
    // Must be followed by at least one whitespace character — guards
    // against false positives like `Test TemplateAlias` matching
    // `Test Template`.
    if !after.starts_with(' ') && !after.starts_with('\t') {
        return None;
    }
    Some(after.trim())
}

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
