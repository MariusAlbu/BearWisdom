// =============================================================================
// languages/cobol/extract.rs  —  COBOL extractor (line scanner)
//
// No tree-sitter grammar (tree-sitter-cobol 0.1.0 is a stub).
// Uses a column-aware line scanner over COBOL fixed-format source.
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — section headings (e.g. "PARAGRAPH-NAME.")
//               and paragraph names in PROCEDURE DIVISION
//   Variable  — data descriptions (01/02/03/77/88 level items in DATA DIVISION)
//   Field     — subordinate data items (level > 01, level != 77)
//
// REFERENCES:
//   Calls     — PERFORM <paragraph-name>
//   Calls+Imports — CALL '<program>'
//   Imports   — COPY <copybook>
//
// COBOL structure:
//   IDENTIFICATION DIVISION.
//   ENVIRONMENT DIVISION.
//   DATA DIVISION.
//     WORKING-STORAGE SECTION.
//       01  WS-FIELD   PIC X(10).
//   PROCEDURE DIVISION.
//     MAIN-PARA.
//       PERFORM SOME-PARA.
//
// Column conventions (fixed format):
//   1–6:   sequence numbers (ignored)
//   7:     indicator (*, /, D, -)
//   8–11:  Area A (division/section/paragraph names, level numbers)
//   12–72: Area B (statements)
//   73+:   identification (ignored)
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use crate::types::ExtractionResult;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Division {
    None,
    Identification,
    Environment,
    Data,
    Procedure,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DataSection {
    None,
    WorkingStorage,
    LocalStorage,
    FileSection,
    LinkageSection,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let mut division = Division::None;
    let mut _data_section = DataSection::None;
    let mut current_para: Option<usize> = None;

    for (lineno, line) in source.lines().enumerate() {
        let row = lineno as u32;

        // Skip blank lines.
        if line.trim().is_empty() {
            continue;
        }

        // Column 7 (index 6) indicator: '*' = comment, '/' = page eject, 'D' = debug.
        let indicator = line.chars().nth(6).unwrap_or(' ');
        if matches!(indicator, '*' | '/') {
            continue;
        }

        // Extract Area A (columns 8-11, index 7-10) and Area B (12+, index 11+).
        let area_a = get_area_a(line);
        let area_b = get_area_b(line);
        let full_stmt = area_a.trim().to_uppercase();

        // Division detection.
        if full_stmt.contains("IDENTIFICATION DIVISION") || full_stmt.contains("ID DIVISION") {
            division = Division::Identification;
            _data_section = DataSection::None;
            continue;
        }
        if full_stmt.contains("ENVIRONMENT DIVISION") {
            division = Division::Environment;
            _data_section = DataSection::None;
            continue;
        }
        if full_stmt.contains("DATA DIVISION") {
            division = Division::Data;
            _data_section = DataSection::None;
            current_para = None;
            continue;
        }
        if full_stmt.contains("PROCEDURE DIVISION") {
            division = Division::Procedure;
            _data_section = DataSection::None;
            current_para = None;
            continue;
        }

        match division {
            Division::Data => {
                // Section detection within DATA DIVISION.
                if full_stmt.ends_with("SECTION.") || full_stmt.ends_with("SECTION") {
                    _data_section = detect_data_section(&full_stmt);
                    continue;
                }

                // Data items: level number at start of Area A.
                // Level numbers: 01-49, 66, 77, 78, 88
                if let Some(level) = parse_level_number(&full_stmt) {
                    let name = extract_data_name(&full_stmt, area_b);
                    if !name.is_empty() && name.to_uppercase() != "FILLER" {
                        let kind = if level == 1 || level == 77 {
                            SymbolKind::Variable
                        } else {
                            SymbolKind::Field
                        };
                        let sig = build_data_sig(level, &name, area_b);
                        let idx = symbols.len();
                        symbols.push(make_symbol(
                            name.clone(),
                            name,
                            kind,
                            row,
                            Some(sig),
                            if level > 1 && level != 77 { current_para } else { None },
                        ));
                        if level == 1 || level == 77 {
                            current_para = Some(idx);
                        }
                    }
                }
            }

            Division::Procedure => {
                // Section heading in PROCEDURE DIVISION: "SECTION-NAME SECTION."
                if full_stmt.ends_with("SECTION.") || full_stmt.ends_with("SECTION") {
                    let name = extract_section_name(&full_stmt);
                    if !name.is_empty() {
                        let idx = symbols.len();
                        symbols.push(make_symbol(
                            name.clone(),
                            name,
                            SymbolKind::Function,
                            row,
                            None,
                            None,
                        ));
                        current_para = Some(idx);
                        continue;
                    }
                }

                // Paragraph: name ends with '.' at column 8+ (Area A), not a keyword.
                if is_paragraph_name(&full_stmt, area_a) {
                    let name = full_stmt.trim_end_matches('.').trim().to_string();
                    if !name.is_empty() && !is_cobol_keyword(&name) {
                        let idx = symbols.len();
                        symbols.push(make_symbol(
                            name.clone(),
                            name,
                            SymbolKind::Function,
                            row,
                            None,
                            None,
                        ));
                        current_para = Some(idx);
                        continue;
                    }
                }

                // Statements in Area B.
                let stmt_upper = area_b.trim().to_uppercase();
                let source_idx = current_para.unwrap_or(0);

                // PERFORM <para-name> [THRU <para-name>] [VARYING ...]
                if let Some(target) = parse_perform(&stmt_upper) {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: target,
                        kind: EdgeKind::Calls,
                        line: row,
                        module: None,
                        chain: None,
                    });
                }

                // CALL '<program>' or CALL "program"
                if let Some(prog) = parse_call(&stmt_upper) {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: prog.clone(),
                        kind: EdgeKind::Calls,
                        line: row,
                        module: None,
                        chain: None,
                    });
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: prog.clone(),
                        kind: EdgeKind::Imports,
                        line: row,
                        module: Some(prog),
                        chain: None,
                    });
                }

                // COPY <copybook>
                if let Some(copybook) = parse_copy(&stmt_upper) {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: copybook.clone(),
                        kind: EdgeKind::Imports,
                        line: row,
                        module: Some(copybook),
                        chain: None,
                    });
                }
            }

            _ => {
                // COPY in non-procedure divisions (DATA, ENVIRONMENT, etc.)
                let stmt_upper = line.trim().to_uppercase();
                let source_idx = current_para.unwrap_or(0);
                if let Some(copybook) = parse_copy(&stmt_upper) {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: copybook.clone(),
                        kind: EdgeKind::Imports,
                        line: row,
                        module: Some(copybook),
                        chain: None,
                    });
                }
            }
        }
    }

    ExtractionResult::new(symbols, refs, false)
}

// ---------------------------------------------------------------------------
// Column extraction helpers (fixed-format COBOL)
// ---------------------------------------------------------------------------

/// Area A: columns 8–11 (0-indexed 7–10). Returns a slice of the line.
fn get_area_a(line: &str) -> &str {
    // Handle both fixed-format and free-format COBOL.
    // If line is shorter than 7 chars, treat the whole line as Area A.
    let bytes = line.as_bytes();
    if bytes.len() <= 7 {
        return line;
    }
    // Skip column 7 indicator, return from col 8 onward (but just the area portion).
    let start = 7; // 0-indexed column 8
    let end = bytes.len().min(72);
    &line[start..end]
}

/// Area B: columns 12–72 (0-indexed 11–71). Returns a slice of the line.
fn get_area_b(line: &str) -> &str {
    let bytes = line.as_bytes();
    if bytes.len() <= 11 {
        return "";
    }
    let end = bytes.len().min(72);
    &line[11..end]
}

// ---------------------------------------------------------------------------
// Data item helpers
// ---------------------------------------------------------------------------

fn detect_data_section(stmt: &str) -> DataSection {
    if stmt.contains("WORKING-STORAGE") {
        DataSection::WorkingStorage
    } else if stmt.contains("LOCAL-STORAGE") {
        DataSection::LocalStorage
    } else if stmt.contains("FILE") {
        DataSection::FileSection
    } else if stmt.contains("LINKAGE") {
        DataSection::LinkageSection
    } else {
        DataSection::None
    }
}

fn parse_level_number(stmt: &str) -> Option<u8> {
    let token = stmt.split_whitespace().next()?;
    token.parse::<u8>().ok().filter(|&n| {
        (n >= 1 && n <= 49) || n == 66 || n == 77 || n == 78 || n == 88
    })
}

fn extract_data_name(stmt: &str, _area_b: &str) -> String {
    // Tokens: "01 WS-FIELD PIC X(10)."
    // The name is the second token.
    let mut tokens = stmt.split_whitespace();
    tokens.next(); // level number
    tokens.next().unwrap_or("").trim_end_matches('.').to_string()
}

fn build_data_sig(level: u8, name: &str, area_b: &str) -> String {
    // Include up to the first PIC clause if present.
    let pic_part = area_b
        .to_uppercase()
        .find("PIC")
        .map(|p| area_b[p..].split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    if pic_part.is_empty() {
        format!("{:02} {}", level, name)
    } else {
        format!("{:02} {} {}", level, name, pic_part)
    }
}

// ---------------------------------------------------------------------------
// Paragraph / section name helpers
// ---------------------------------------------------------------------------

fn extract_section_name(stmt: &str) -> String {
    // "SECTION-NAME SECTION." → "SECTION-NAME"
    stmt.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_string()
}

fn is_paragraph_name(stmt: &str, area_a: &str) -> bool {
    // A paragraph name is a single token ending in '.' at Area A position
    // (i.e., not preceded by whitespace in columns 8-11).
    let trimmed = stmt.trim();
    if !trimmed.ends_with('.') {
        return false;
    }
    // Only one token (the paragraph name).
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() != 1 {
        return false;
    }
    // Must be in Area A (starts without leading whitespace relative to col 8).
    !area_a.starts_with(' ')
}

fn is_cobol_keyword(name: &str) -> bool {
    matches!(
        name,
        "STOP" | "EXIT" | "MOVE" | "ADD" | "SUBTRACT" | "MULTIPLY" | "DIVIDE"
        | "COMPUTE" | "IF" | "ELSE" | "END-IF" | "EVALUATE" | "WHEN" | "END-EVALUATE"
        | "PERFORM" | "CALL" | "COPY" | "DISPLAY" | "ACCEPT" | "READ" | "WRITE"
        | "OPEN" | "CLOSE" | "INITIALIZE" | "INSPECT" | "STRING" | "UNSTRING"
        | "SORT" | "MERGE" | "RETURN" | "RELEASE" | "CONTINUE" | "NEXT"
        | "SENTENCE" | "GO" | "ALTER" | "RUN" | "GOBACK" | "END-PROGRAM"
    )
}

// ---------------------------------------------------------------------------
// Statement parsers
// ---------------------------------------------------------------------------

/// Parse "PERFORM <para-name>" and return the target paragraph name.
/// Also handles "PERFORM <para-name> THRU <para-name>".
fn parse_perform(stmt: &str) -> Option<String> {
    let upper = stmt.trim();
    if !upper.starts_with("PERFORM ") && upper != "PERFORM" {
        return None;
    }
    let rest = upper["PERFORM".len()..].trim();
    // Inline PERFORM (PERFORM ... END-PERFORM) — skip, no target name.
    if rest.starts_with("UNTIL") || rest.starts_with("VARYING") || rest.starts_with("WITH") {
        return None;
    }
    let target = rest
        .split_whitespace()
        .next()?
        .trim_end_matches('.')
        .to_string();
    if target.is_empty() || is_cobol_keyword(&target) {
        return None;
    }
    Some(target)
}

/// Parse "CALL '<program>'" or "CALL \"program\"".
fn parse_call(stmt: &str) -> Option<String> {
    let upper = stmt.trim();
    if !upper.starts_with("CALL ") {
        return None;
    }
    let rest = upper["CALL".len()..].trim();
    // Strip surrounding quotes from string literal.
    let name = rest
        .trim_start_matches('\'')
        .trim_start_matches('"')
        .split(|c| c == '\'' || c == '"' || c == ' ')
        .next()?
        .to_string();
    if name.is_empty() {
        return None;
    }
    Some(name)
}

/// Parse "COPY <copybook>" or "COPY <copybook> [IN/OF <library>]".
fn parse_copy(stmt: &str) -> Option<String> {
    let upper = stmt.trim();
    if !upper.starts_with("COPY ") {
        return None;
    }
    let rest = upper["COPY".len()..].trim();
    let name = rest
        .split_whitespace()
        .next()?
        .trim_end_matches('.')
        .to_string();
    if name.is_empty() {
        return None;
    }
    Some(name)
}

// ---------------------------------------------------------------------------
// Symbol factory
// ---------------------------------------------------------------------------

fn make_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    line: u32,
    signature: Option<String>,
    parent_index: Option<usize>,
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
        parent_index,
    }
}
