// =============================================================================
// languages/gleam/extract.rs  —  Gleam extractor (no tree-sitter grammar)
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — `pub fn name(...)` / `fn name(...)` (pub → Public)
//   Function  — `@external(erlang, ...) pub fn name(...)` (FFI)
//   Enum      — `pub type Name { ... }` / `type Name { ... }` (custom type/ADT)
//   TypeAlias — `pub type Name = OtherType`
//   Variable  — `pub const name = ...` / `const name = ...`
//
// REFERENCES:
//   Imports   — `import module` / `import module.{symbol}`
//   Calls     — `value |> func(...)` pipelines (emit Calls for `func`)
//
// Gleam is a functional language on the BEAM (Erlang) VM. Declarations are
// always at the top level (no block nesting for definitions).
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

pub fn extract(source: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;
    let mut skip_external = false;

    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim();

        if trimmed.is_empty() || trimmed.starts_with("//") {
            i += 1;
            skip_external = false;
            continue;
        }

        // @external attribute — next fn is FFI
        if trimmed.starts_with("@external(") {
            skip_external = true;
            i += 1;
            continue;
        }

        // import
        if trimmed.starts_with("import ") {
            let module = parse_import(trimmed);
            if let Some(name) = module {
                refs.push(ExtractedRef {
                    source_symbol_index: 0,
                    target_name: name,
                    kind: EdgeKind::Imports,
                    line: i as u32,
                    module: None,
                    chain: None,
                });
            }
            skip_external = false;
            i += 1;
            continue;
        }

        let (is_pub, rest) = strip_pub(trimmed);
        let vis = if is_pub { Visibility::Public } else { Visibility::Private };

        // fn declaration
        if rest.starts_with("fn ") {
            if let Some(name) = parse_fn_name(rest) {
                let start = i as u32;
                let end = find_brace_end(&lines, i);
                let fn_idx = symbols.len();
                symbols.push(make_sym(name, SymbolKind::Function, vis, start, end));
                // Extract pipe calls from the function body
                let body_start = i + 1;
                let body_end = end as usize;
                if body_end > body_start {
                    extract_pipe_calls(&lines[body_start..body_end], body_start, fn_idx, &mut refs);
                }
                i = end as usize + 1;
            } else {
                i += 1;
            }
            skip_external = false;
            continue;
        }

        // type declaration
        if rest.starts_with("type ") {
            if let Some((name, kind)) = parse_type_decl(rest) {
                let start = i as u32;
                let end = if kind == SymbolKind::Enum {
                    find_brace_end(&lines, i)
                } else {
                    start // TypeAlias is single-line
                };
                symbols.push(make_sym(name, kind, vis, start, end));
                i = end as usize + 1;
            } else {
                i += 1;
            }
            skip_external = false;
            continue;
        }

        // const declaration
        if rest.starts_with("const ") {
            if let Some(name) = parse_const_name(rest) {
                let start = i as u32;
                symbols.push(make_sym(name, SymbolKind::Variable, vis, start, start));
            }
            skip_external = false;
            i += 1;
            continue;
        }

        let _ = skip_external;
        skip_external = false;
        i += 1;
    }

    ExtractionResult::new(symbols, refs, false)
}

// ---------------------------------------------------------------------------
// Line parsers
// ---------------------------------------------------------------------------

fn parse_import(line: &str) -> Option<String> {
    let rest = line.strip_prefix("import ")?;
    // Module name ends at `{`, `/`, or end of line
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '/')
        .collect();
    // Convert `/` to `.` for display
    let name = name.replace('/', ".");
    if name.is_empty() { return None; }
    Some(name)
}

fn strip_pub(s: &str) -> (bool, &str) {
    if let Some(r) = s.strip_prefix("pub ") {
        return (true, r.trim_start());
    }
    (false, s)
}

fn parse_fn_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("fn ")?;
    let name: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { return None; }
    Some(name)
}

fn parse_type_decl(line: &str) -> Option<(String, SymbolKind)> {
    let rest = line.strip_prefix("type ")?;
    let rest = rest.trim_start();
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { return None; }
    let after = rest[name.len()..].trim_start();
    let kind = if after.starts_with('{') || after.is_empty() {
        SymbolKind::Enum   // Custom type / ADT
    } else if after.starts_with('=') {
        SymbolKind::TypeAlias
    } else {
        SymbolKind::Enum   // Parametrized custom type
    };
    Some((name, kind))
}

fn parse_const_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("const ")?;
    let name: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { return None; }
    Some(name)
}

// ---------------------------------------------------------------------------
// Pipe call extraction
// ---------------------------------------------------------------------------

/// Scan body lines for `|>` pipeline calls and emit Calls edges.
fn extract_pipe_calls(
    body: &[&str],
    base_line: usize,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    for (k, &line) in body.iter().enumerate() {
        if !line.contains("|>") {
            continue;
        }
        let line_num = (base_line + k) as u32;
        // Each `|> func_name(` after the pipe is a call target.
        let mut rest = line;
        while let Some(pos) = rest.find("|>") {
            rest = rest[pos + 2..].trim_start();
            // Extract the identifier after `|>`
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: line_num,
                    module: None,
                    chain: None,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_sym(name: String, kind: SymbolKind, vis: Visibility, start: u32, end: u32) -> ExtractedSymbol {
    ExtractedSymbol {
        qualified_name: name.clone(),
        name,
        kind,
        visibility: Some(vis),
        start_line: start,
        end_line: end,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn find_brace_end(lines: &[&str], start: usize) -> u32 {
    let mut depth = 0i32;
    for (k, &line) in lines[start..].iter().enumerate() {
        for ch in line.chars() {
            if ch == '{' { depth += 1; }
            else if ch == '}' {
                depth -= 1;
                if depth <= 0 {
                    return (start + k) as u32;
                }
            }
        }
    }
    start as u32
}
