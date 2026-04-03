// =============================================================================
// languages/starlark/extract.rs  —  Starlark / Bazel BUILD extractor
//
// No tree-sitter grammar — uses a line-oriented scanner.
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — `def name(...):`
//   Variable  — `name = value` (module-level, non-rule, non-function)
//   Function  — `name = rule(...)` / `name = macro(...)` / `name = aspect(...)`
//   Struct    — `name = provider(...)` / `name = struct(...)`
//   Test      — assignment where RHS call name ends in `_test`
//
// REFERENCES:
//   Imports   — `load("label", "sym1", sym2="sym3")` → module label + symbol names
//   Calls     — function calls at statement level and inside def bodies
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use crate::types::ExtractionResult;

// Rule-like builtins that define new rule types
const RULE_FUNCS: &[&str] = &["rule", "macro", "aspect", "repository_rule"];
// Struct-like builtins
const STRUCT_FUNCS: &[&str] = &["provider", "struct"];
// Keywords to skip when scanning for calls
const STARLARK_KWS: &[&str] = &[
    "if", "else", "for", "while", "def", "return", "and", "or",
    "not", "in", "pass", "break", "continue", "load", "True", "False", "None",
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();

        // Only process top-level statements (indent == 0)
        if indent != 0 {
            i += 1;
            continue;
        }

        if trimmed.starts_with("def ") {
            // Function definition
            let rest = &trimmed[4..];
            if let Some(name) = extract_def_name(rest) {
                let params = extract_def_params(rest);
                let sig = format!("def {}({})", name, params);
                let idx = symbols.len();
                symbols.push(make_symbol(
                    name.clone(), name, SymbolKind::Function, i as u32, Some(sig),
                ));
                // Scan body for calls
                let body_end = find_block_end(&lines, i + 1, 4);
                extract_body_calls(&lines, i + 1, body_end, &mut refs, idx);
                i = body_end;
                continue;
            }
        } else if trimmed.starts_with("load(") {
            // load("label", "sym1", ...)
            extract_load(trimmed, i as u32, symbols.len().saturating_sub(1), &mut refs);
        } else if let Some(eq_pos) = find_assignment(trimmed) {
            let name = trimmed[..eq_pos].trim().to_string();
            let rhs = trimmed[eq_pos + 1..].trim();

            if let Some(callee) = extract_call_name(rhs) {
                if RULE_FUNCS.contains(&callee.as_str()) {
                    symbols.push(make_symbol(
                        name.clone(), name.clone(), SymbolKind::Function, i as u32,
                        Some(format!("{} = {}(...)", name, callee)),
                    ));
                } else if STRUCT_FUNCS.contains(&callee.as_str()) {
                    symbols.push(make_symbol(
                        name.clone(), name.clone(), SymbolKind::Struct, i as u32,
                        Some(format!("{} = {}(...)", name, callee)),
                    ));
                } else if callee.ends_with("_test") {
                    symbols.push(make_symbol(
                        name.clone(), name.clone(), SymbolKind::Test, i as u32,
                        Some(format!("{} = {}(...)", name, callee)),
                    ));
                } else {
                    // Generic variable assignment with call RHS
                    let source_idx = symbols.len().saturating_sub(1);
                    symbols.push(make_symbol(
                        name.clone(), name, SymbolKind::Variable, i as u32, None,
                    ));
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: callee,
                        kind: EdgeKind::Calls,
                        line: i as u32,
                        module: None,
                        chain: None,
                    });
                }
            } else {
                // Plain variable assignment
                symbols.push(make_symbol(
                    name.clone(), name, SymbolKind::Variable, i as u32, None,
                ));
            }
        } else if trimmed.contains('(') {
            // Bare call statement (e.g., a rule invocation in BUILD file)
            if let Some(callee) = extract_call_name(trimmed) {
                let source_idx = symbols.len().saturating_sub(1);
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: callee,
                    kind: EdgeKind::Calls,
                    line: i as u32,
                    module: None,
                    chain: None,
                });
            }
        }

        i += 1;
    }

    ExtractionResult::new(symbols, refs, false)
}

// ---------------------------------------------------------------------------
// load() extraction
// ---------------------------------------------------------------------------

fn extract_load(line: &str, lineno: u32, source_idx: usize, refs: &mut Vec<ExtractedRef>) {
    // Crude parse: load("label", "sym1", alias="sym2", ...)
    let inside = match line.find('(').and_then(|s| line.rfind(')').map(|e| (s, e))) {
        Some((s, e)) if s < e => &line[s + 1..e],
        _ => return,
    };

    let parts = split_load_args(inside);
    if parts.is_empty() {
        return;
    }

    let module_label = parts[0].trim().trim_matches(|c| c == '"' || c == '\'').to_string();
    if module_label.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index: source_idx,
        target_name: module_label.clone(),
        kind: EdgeKind::Imports,
        line: lineno,
        module: Some(module_label.clone()),
        chain: None,
    });

    // Each remaining arg is a symbol import
    for part in &parts[1..] {
        let part = part.trim();
        let sym = if let Some(eq) = part.find('=') {
            part[eq + 1..].trim().trim_matches(|c| c == '"' || c == '\'')
        } else {
            part.trim_matches(|c| c == '"' || c == '\'')
        };
        if !sym.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: sym.to_string(),
                kind: EdgeKind::Imports,
                line: lineno,
                module: Some(module_label.clone()),
                chain: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Body call extraction
// ---------------------------------------------------------------------------

fn extract_body_calls(
    lines: &[&str],
    start: usize,
    end: usize,
    refs: &mut Vec<ExtractedRef>,
    source_idx: usize,
) {
    for (offset, &line) in lines[start..end].iter().enumerate() {
        let lineno = (start + offset) as u32;
        for call in find_calls_in_line(line) {
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: call,
                kind: EdgeKind::Calls,
                line: lineno,
                module: None,
                chain: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_def_name(rest: &str) -> Option<String> {
    let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
    let name = &rest[..name_end];
    if name.is_empty() { None } else { Some(name.to_string()) }
}

fn extract_def_params(rest: &str) -> String {
    if let Some(start) = rest.find('(') {
        if let Some(end) = rest.rfind(')') {
            if start < end {
                return rest[start + 1..end].trim().to_string();
            }
        }
    }
    String::new()
}

fn find_assignment(line: &str) -> Option<usize> {
    // Find `=` that is not `==`, `!=`, `<=`, `>=`, `+=`, `-=`, etc.
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'=' {
            if i > 0 && matches!(bytes[i - 1], b'!' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/' | b'=') {
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                continue;
            }
            let lhs = line[..i].trim();
            if !lhs.is_empty() && lhs.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Some(i);
            }
        }
    }
    None
}

fn extract_call_name(rhs: &str) -> Option<String> {
    let trimmed = rhs.trim_start();
    let paren = trimmed.find('(')?;
    let callee = trimmed[..paren].trim();
    if callee.is_empty() || !is_simple_ident(callee) {
        return None;
    }
    Some(callee.to_string())
}

fn find_calls_in_line(line: &str) -> Vec<String> {
    let mut calls = Vec::new();
    let mut chars = line.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if c.is_alphabetic() || c == '_' {
            let mut end = start + c.len_utf8();
            while let Some(&(pos, nc)) = chars.peek() {
                if nc.is_alphanumeric() || nc == '_' || nc == '.' {
                    end = pos + nc.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let name = &line[start..end];
            // Check if followed by `(`
            let rest = line[end..].trim_start();
            if rest.starts_with('(') && !STARLARK_KWS.contains(&name) {
                calls.push(name.to_string());
            }
        }
    }
    calls
}

fn find_block_end(lines: &[&str], start: usize, min_indent: usize) -> usize {
    for i in start..lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            continue;
        }
        if leading_spaces(line) < min_indent {
            return i;
        }
    }
    lines.len()
}

fn split_load_args(inside: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut quote_char: u8 = 0;
    let bytes = inside.as_bytes();
    for i in 0..bytes.len() {
        match bytes[i] {
            b'"' | b'\'' if !in_quote => { in_quote = true; quote_char = bytes[i]; }
            c if in_quote && c == quote_char => { in_quote = false; }
            b'(' | b'[' | b'{' if !in_quote => { depth += 1; }
            b')' | b']' | b'}' if !in_quote => { depth -= 1; }
            b',' if !in_quote && depth == 0 => {
                parts.push(&inside[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < inside.len() {
        parts.push(&inside[start..]);
    }
    parts
}

fn leading_spaces(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn is_simple_ident(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_')
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
