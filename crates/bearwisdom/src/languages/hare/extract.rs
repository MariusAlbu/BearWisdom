// =============================================================================
// languages/hare/extract.rs  —  Hare extractor (no tree-sitter grammar)
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — `fn name(...)` (export → Public)
//   Test      — `@test fn name(...)`
//   Struct    — `type Name = struct { ... }`
//   Enum      — `type Name = enum { ... }`
//   TypeAlias — `type Name = OtherType;`
//   Variable  — `def Name: type = value;` (compile-time constant)
//   Variable  — `let Name: type = value;` (global)
//
// REFERENCES:
//   Imports   — `use module::path;`
//
// Hare declarations are typically at the top level. The language uses
// `export` as a visibility modifier and C-like block syntax.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

pub fn extract(source: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim();

        if trimmed.is_empty() || trimmed.starts_with("//") {
            i += 1;
            continue;
        }

        // use statement
        if let Some(target) = parse_use(trimmed) {
            refs.push(ExtractedRef {
                source_symbol_index: 0,
                target_name: target,
                kind: EdgeKind::Imports,
                line: i as u32,
                module: None,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
});
            i += 1;
            continue;
        }

        // @test fn
        if trimmed.starts_with("@test") {
            // Find the fn on this or the next line
            let fn_line = if trimmed.contains("fn ") {
                trimmed
            } else if i + 1 < lines.len() && lines[i + 1].trim().starts_with("fn ") {
                i += 1;
                lines[i].trim()
            } else {
                i += 1;
                continue;
            };
            // Strip `@test ` (or `@test\t`) prefix so that single-line
            // `@test fn name()` is handled by parse_fn_name.
            let fn_part = if fn_line.starts_with("@test") {
                // Skip past "@test" and any whitespace to reach "fn ..."
                let after = fn_line["@test".len()..].trim_start();
                after
            } else {
                fn_line
            };
            if let Some(name) = parse_fn_name(fn_part) {
                let start = i as u32;
                let end = find_brace_end(&lines, i);
                let mut sym = make_sym(name, SymbolKind::Test, Visibility::Private, start, end);
                sym.signature = Some(fn_line.to_string());
                symbols.push(sym);
                i = end as usize + 1;
                continue;
            }
        }

        // fn declaration
        {
            let (is_export, rest) = strip_export(trimmed);
            if rest.starts_with("fn ") {
                if let Some(name) = parse_fn_name(rest) {
                    let start = i as u32;
                    let end = find_brace_end(&lines, i);
                    let vis = if is_export { Visibility::Public } else { Visibility::Private };
                    symbols.push(make_sym(name, SymbolKind::Function, vis, start, end));
                    i = end as usize + 1;
                    continue;
                }
            }

            // type declaration
            if rest.starts_with("type ") {
                if let Some((name, kind)) = parse_type_decl(rest) {
                    let start = i as u32;
                    let end = if kind == SymbolKind::Struct || kind == SymbolKind::Enum {
                        find_brace_end(&lines, i)
                    } else {
                        find_semicolon_end(&lines, i)
                    };
                    let vis = if is_export { Visibility::Public } else { Visibility::Private };
                    symbols.push(make_sym(name, kind, vis, start, end));
                    i = end as usize + 1;
                    continue;
                }
            }

            // def / let
            if rest.starts_with("def ") || rest.starts_with("let ") {
                if let Some(name) = parse_def_let_name(rest) {
                    let start = i as u32;
                    let end = find_semicolon_end(&lines, i);
                    let vis = if is_export { Visibility::Public } else { Visibility::Private };
                    symbols.push(make_sym(name, SymbolKind::Variable, vis, start, end));
                    i = end as usize + 1;
                    continue;
                }
            }
        }

        i += 1;
    }

    ExtractionResult::new(symbols, refs, false)
}

// ---------------------------------------------------------------------------
// Line parsers
// ---------------------------------------------------------------------------

fn parse_use(line: &str) -> Option<String> {
    let rest = line.strip_prefix("use ")?;
    // Strip trailing `;` and any `{...}` selective imports
    let module = rest
        .split('{')
        .next()
        .unwrap_or(rest)
        .trim_end_matches(';')
        .trim()
        .to_string();
    if module.is_empty() { return None; }
    Some(module)
}

fn strip_export(s: &str) -> (bool, &str) {
    if let Some(r) = s.strip_prefix("export ") {
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
    // `Name = rhs`
    let eq_pos = rest.find('=')?;
    let name_part = rest[..eq_pos].trim();
    let name: String = name_part
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { return None; }
    let rhs = rest[eq_pos + 1..].trim();
    let kind = if rhs.starts_with("struct") || rhs.starts_with("nullable") {
        SymbolKind::Struct
    } else if rhs.starts_with("enum") {
        SymbolKind::Enum
    } else if rhs.starts_with("union") {
        SymbolKind::Struct
    } else {
        SymbolKind::TypeAlias
    };
    Some((name, kind))
}

fn parse_def_let_name(line: &str) -> Option<String> {
    let rest = if let Some(r) = line.strip_prefix("def ") {
        r
    } else if let Some(r) = line.strip_prefix("let ") {
        r
    } else {
        return None;
    };
    let name: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { return None; }
    Some(name)
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

fn find_semicolon_end(lines: &[&str], start: usize) -> u32 {
    for (k, &line) in lines[start..].iter().enumerate() {
        if line.contains(';') {
            return (start + k) as u32;
        }
    }
    start as u32
}
