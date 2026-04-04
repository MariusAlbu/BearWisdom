// =============================================================================
// languages/odin/extract.rs  —  Odin extractor (no tree-sitter grammar)
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — `name :: proc(...)`
//   Struct    — `name :: struct { ... }`
//   Enum      — `name :: enum { ... }`
//   Struct    — `name :: union { ... }` (tagged union)
//   Variable  — `name :: value` / `name : Type = value` (plain constants)
//
// REFERENCES:
//   Imports   — `import "path"` / `import name "path"`
//   TypeRef   — `using expr` → the type name being composed
//
// Odin uses `::` for constant declarations and `:=` / `: Type =` for vars.
// All top-level declarations begin at column 0 (no indentation requirement,
// but that's the overwhelmingly common style in Odin packages).
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

        // import declaration
        if trimmed.starts_with("import ") {
            if let Some(target) = parse_import(trimmed) {
                refs.push(ExtractedRef {
                    source_symbol_index: 0,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: i as u32,
                    module: None,
                    chain: None,
                });
            }
            i += 1;
            continue;
        }

        // using statement → TypeRef
        if trimmed.starts_with("using ") {
            let name_part = trimmed["using ".len()..].trim();
            // e.g. `using BaseStruct` or `using pkg.Type`
            let type_name: String = name_part
                .split(|c: char| c == ';' || c == '\n' || c == ' ')
                .next()
                .unwrap_or("")
                .to_string();
            if !type_name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: 0,
                    target_name: type_name,
                    kind: EdgeKind::TypeRef,
                    line: i as u32,
                    module: None,
                    chain: None,
                });
            }
            i += 1;
            continue;
        }

        // Constant / type declarations: `Name :: proc/struct/enum/union/...`
        if let Some((name, kind, vis)) = parse_decl(trimmed) {
            let start = i as u32;
            // For block types, scan for the closing brace.
            let end = if kind != SymbolKind::Variable {
                find_brace_end(&lines, i)
            } else {
                start
            };
            symbols.push(make_sym(name, kind, vis, start, end));
            i = end as usize + 1;
            continue;
        }

        i += 1;
    }

    ExtractionResult::new(symbols, refs, false)
}

// ---------------------------------------------------------------------------
// Line parsers
// ---------------------------------------------------------------------------

/// Parse `import "path"` or `import alias "path"`.
fn parse_import(line: &str) -> Option<String> {
    let rest = line.strip_prefix("import ")?.trim_start();
    // Check if next token is an alias identifier (no quotes)
    if rest.starts_with('"') {
        let inner = rest.strip_prefix('"')?;
        let end = inner.find('"').unwrap_or(inner.len());
        return Some(inner[..end].to_string());
    }
    // `import alias "path"` — skip alias, grab path
    let after_alias = rest.find('"')?;
    let path_start = &rest[after_alias + 1..];
    let path_end = path_start.find('"').unwrap_or(path_start.len());
    Some(path_start[..path_end].to_string())
}

/// Try to parse `Name :: rhs` or `Name : Type : rhs`.
fn parse_decl(line: &str) -> Option<(String, SymbolKind, Visibility)> {
    // Find `::` separator
    let cc_pos = line.find("::")?;
    let name_part = line[..cc_pos].trim();

    // Name must be a plain identifier (no spaces, no type annotations yet).
    // Odin allows `name: type :` but we handle simple `name ::` here.
    let name: String = name_part
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() || name != name_part {
        return None;
    }

    let rhs = line[cc_pos + 2..].trim();

    // Determine what's being declared.
    let kind = if rhs.starts_with("proc") {
        SymbolKind::Function
    } else if rhs.starts_with("struct") {
        SymbolKind::Struct
    } else if rhs.starts_with("enum") {
        SymbolKind::Enum
    } else if rhs.starts_with("union") {
        SymbolKind::Struct // tagged union → Struct
    } else if rhs.starts_with("bit_set") {
        SymbolKind::Enum
    } else {
        // Plain constant or type alias
        SymbolKind::Variable
    };

    // Odin has no access modifiers at the language level (everything in a
    // package is accessible); we mark exported-by-convention as Public.
    let vis = if name.starts_with('_') { Visibility::Private } else { Visibility::Public };

    Some((name, kind, vis))
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

/// Scan forward to find the matching `}` for the opening `{` on or after `start`.
fn find_brace_end(lines: &[&str], start: usize) -> u32 {
    let mut depth = 0i32;
    let mut end = start as u32;
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
        if depth > 0 {
            end = (start + k) as u32;
        }
    }
    end
}
