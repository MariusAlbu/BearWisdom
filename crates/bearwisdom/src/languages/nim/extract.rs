// =============================================================================
// languages/nim/extract.rs  —  Nim extractor (no tree-sitter grammar)
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — `proc name(...)`, `func name(...)`, `template name(...)`,
//               `macro name(...)`, `iterator name(...)`, `converter name(...)`
//   Method    — `method name(...)`
//   Struct    — `type Name = object`
//   Enum      — `type Name = enum`
//   Interface — `type Name = concept`
//   TypeAlias — `type Name = OtherType` (not object/enum/concept)
//
// REFERENCES:
//   Imports   — `import module`, `import module, module2`, `from module import ...`
//
// This is a single-pass line scanner. Nim's indentation-based syntax makes
// full block tracking impractical without a grammar, so we scan top-level
// declarations only (zero indentation).
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

pub fn extract(source: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    // Track whether we're inside a type section (for type declarations).
    let mut in_type_section = false;
    // Indentation of the type section body (first indented line after `type`)
    let mut type_section_indent: usize = 0;

    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        let indent = leading_spaces(raw);

        // Detect leaving a type section when indentation resets to ≤ section start.
        if in_type_section && indent <= type_section_indent && !trimmed.is_empty() {
            // Still inside if this line has more indent than the `type` keyword.
            // `type` is at indent 0, so body is at indent > 0.
            if indent == 0 {
                in_type_section = false;
            }
        }

        // Only process top-level declarations (indent == 0) or type section body.
        if indent == 0 {
            // Top-level keyword
            if trimmed == "type" || trimmed.starts_with("type ") && trimmed.ends_with(':') {
                in_type_section = true;
                type_section_indent = 0;
                i += 1;
                // Peek at the next non-blank line to determine body indent
                let mut j = i;
                while j < lines.len() && lines[j].trim().is_empty() {
                    j += 1;
                }
                if j < lines.len() {
                    type_section_indent = leading_spaces(lines[j]);
                }
                continue;
            }

            // Procedure / function / etc.
            if let Some((name, kind, vis)) = parse_proc_line(trimmed) {
                let start = i as u32;
                let end = find_block_end(&lines, i);
                symbols.push(make_sym(name, kind, vis, start, end));
                i = end as usize + 1;
                continue;
            }

            // import / from ... import
            if let Some(import_refs) = parse_import_line(trimmed, i as u32) {
                refs.extend(import_refs);
                i += 1;
                continue;
            }

            // Single-line type declaration: `type Name = ...`
            if let Some((tname, tkind)) = parse_type_decl_line(trimmed) {
                let line = i as u32;
                symbols.push(make_sym(tname, tkind, Visibility::Public, line, line));
                i += 1;
                continue;
            }
        } else if in_type_section && indent >= type_section_indent && type_section_indent > 0 {
            // Inside a `type` section body — each indented `Name = ...` is a type.
            if let Some((tname, tkind)) = parse_type_section_entry(trimmed) {
                let line = i as u32;
                let end = find_type_block_end(&lines, i, indent);
                symbols.push(make_sym(tname, tkind, Visibility::Public, line, end));
                i = end as usize + 1;
                continue;
            }
        }

        i += 1;
    }

    ExtractionResult::new(symbols, refs, false)
}

// ---------------------------------------------------------------------------
// Line parsers
// ---------------------------------------------------------------------------

/// Parse a proc/func/method/template/macro/iterator/converter line.
/// Returns (name, kind, visibility).
fn parse_proc_line(line: &str) -> Option<(String, SymbolKind, Visibility)> {
    let (is_pub, rest) = strip_pub(line);

    let (kind, rest) = if let Some(r) = rest.strip_prefix("proc ") {
        (SymbolKind::Function, r)
    } else if let Some(r) = rest.strip_prefix("func ") {
        (SymbolKind::Function, r)
    } else if let Some(r) = rest.strip_prefix("method ") {
        (SymbolKind::Method, r)
    } else if let Some(r) = rest.strip_prefix("template ") {
        (SymbolKind::Function, r)
    } else if let Some(r) = rest.strip_prefix("macro ") {
        (SymbolKind::Function, r)
    } else if let Some(r) = rest.strip_prefix("iterator ") {
        (SymbolKind::Function, r)
    } else if let Some(r) = rest.strip_prefix("converter ") {
        (SymbolKind::Function, r)
    } else {
        return None;
    };

    let rest = rest.trim_start();
    // Name ends at '(' or ':' or '[' or whitespace — also strip * (export marker)
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return None;
    }

    let vis = if is_pub { Visibility::Public } else { Visibility::Private };
    Some((name, kind, vis))
}

/// Parse top-level `type Name = ...` (single line, no section).
/// Returns (name, kind).
fn parse_type_decl_line(line: &str) -> Option<(String, SymbolKind)> {
    let rest = line.strip_prefix("type ")?;
    let rest = rest.trim_start();
    parse_type_rhs(rest)
}

/// Parse an entry inside a `type` section (indented `Name = ...`).
fn parse_type_section_entry(trimmed: &str) -> Option<(String, SymbolKind)> {
    // Skip lines that are continuations of a previous definition (start with `|` or whitespace).
    if trimmed.starts_with('|') || trimmed.starts_with('#') {
        return None;
    }
    parse_type_rhs(trimmed)
}

/// Parse `Name = rhs` → (name, kind).
fn parse_type_rhs(s: &str) -> Option<(String, SymbolKind)> {
    // Name may have generic params: `Name[T]` or `Name`
    let name: String = s
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return None;
    }
    // Skip optional generic params, then look for `=`
    let after_name = s[name.len()..].trim_start();
    let after_name = if after_name.starts_with('[') {
        // Skip to matching `]`
        let mut depth = 0usize;
        let mut end = 0;
        for (pos, ch) in after_name.char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = pos + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        after_name[end..].trim_start()
    } else {
        after_name
    };

    let rhs = after_name.strip_prefix('=')?.trim_start();

    let kind = if rhs.starts_with("object") || rhs.starts_with("ref object") || rhs.starts_with("ptr object") {
        SymbolKind::Struct
    } else if rhs.starts_with("enum") {
        SymbolKind::Enum
    } else if rhs.starts_with("concept") {
        SymbolKind::Interface
    } else if rhs.starts_with("tuple") {
        SymbolKind::Struct
    } else {
        SymbolKind::TypeAlias
    };

    Some((name, kind))
}

/// Parse `import` and `from ... import` lines.
/// Returns a list of `ExtractedRef` with `EdgeKind::Imports`.
///
/// Handled forms:
///   `import os, strformat`
///   `import std/sequtils`
///   `import std/[sequtils, strutils, options]`     ← bracketed group
///   `import pkg/foo/[bar, baz]`                    ← prefixed bracketed group
///   `from std/strformat import fmt`
///   `import other as O`
fn parse_import_line(line: &str, line_num: u32) -> Option<Vec<ExtractedRef>> {
    if let Some(rest) = line.strip_prefix("import ") {
        let names = expand_nim_imports(rest);
        if names.is_empty() { return None; }
        let modules: Vec<ExtractedRef> = names
            .into_iter()
            .map(|n| ExtractedRef {
                source_symbol_index: 0,
                target_name: n,
                kind: EdgeKind::Imports,
                line: line_num,
                module: None,
                chain: None,
                byte_offset: 0,
                namespace_segments: Vec::new(),
            })
            .collect();
        return Some(modules);
    }

    if let Some(rest) = line.strip_prefix("from ") {
        // `from module import symbol, symbol2` — `module` is typically a single
        // module path (`std/strformat`) but the bracketed group form is also
        // legal as the source. Reuse the same expander.
        let module_part = match rest.find("import") {
            Some(idx) => rest[..idx].trim(),
            None => rest.split_whitespace().next().unwrap_or(""),
        };
        if module_part.is_empty() { return None; }
        let names = expand_nim_imports(module_part);
        if names.is_empty() { return None; }
        let modules: Vec<ExtractedRef> = names
            .into_iter()
            .map(|n| ExtractedRef {
                source_symbol_index: 0,
                target_name: n,
                kind: EdgeKind::Imports,
                line: line_num,
                module: None,
                chain: None,
                byte_offset: 0,
                namespace_segments: Vec::new(),
            })
            .collect();
        return Some(modules);
    }

    None
}

/// Expand a Nim import RHS into individual module paths. Splits on commas at
/// the top level, and expands `prefix/[a, b, c]` into `prefix/a`, `prefix/b`,
/// `prefix/c`. Strips trailing `as alias` clauses.
fn expand_nim_imports(rest: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut buf = String::new();
    for ch in rest.chars() {
        match ch {
            '[' => { depth += 1; buf.push(ch); }
            ']' => { depth -= 1; buf.push(ch); }
            ',' if depth == 0 => {
                expand_one_import(buf.trim(), &mut out);
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    expand_one_import(buf.trim(), &mut out);
    out
}

fn expand_one_import(item: &str, out: &mut Vec<String>) {
    let item = item.trim();
    if item.is_empty() { return; }
    // Strip `as alias` suffix.
    let item = item.split(" as ").next().unwrap_or(item).trim();
    // Bracket-group form: `prefix/[a, b]`.
    if let (Some(open), Some(close)) = (item.find('['), item.rfind(']')) {
        if open < close {
            let prefix = item[..open].trim().trim_end_matches('/');
            let inside = &item[open + 1..close];
            for sub in inside.split(',') {
                let sub = sub.trim();
                if sub.is_empty() { continue; }
                let sub = sub.split(" as ").next().unwrap_or(sub).trim();
                if prefix.is_empty() {
                    out.push(sub.to_string());
                } else {
                    out.push(format!("{prefix}/{sub}"));
                }
            }
            return;
        }
    }
    // Plain `name` or `prefix/name`.
    out.push(item.to_string());
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn strip_pub(s: &str) -> (bool, &str) {
    if let Some(r) = s.strip_prefix("pub ") {
        return (true, r.trim_start());
    }
    // Nim uses `*` after the name as an export marker, not `pub`.
    // We treat all proc/func as Public since visibility is post-name.
    (false, s)
}

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

fn leading_spaces(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Find where a block started at `start` ends.
/// In Nim, blocks end when indentation returns to the base level.
/// Simple heuristic: scan forward until a line at indent ≤ start's indent.
fn find_block_end(lines: &[&str], start: usize) -> u32 {
    let base_indent = leading_spaces(lines[start]);
    let mut end = start as u32;
    for (k, &line) in lines[start + 1..].iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let ind = leading_spaces(line);
        if ind <= base_indent {
            return end;
        }
        end = (start + 1 + k) as u32;
    }
    end
}

/// Like `find_block_end` but relative to a known `block_indent` level.
fn find_type_block_end(lines: &[&str], start: usize, block_indent: usize) -> u32 {
    let mut end = start as u32;
    for (k, &line) in lines[start + 1..].iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let ind = leading_spaces(line);
        if ind <= block_indent {
            return end;
        }
        end = (start + 1 + k) as u32;
    }
    end
}
