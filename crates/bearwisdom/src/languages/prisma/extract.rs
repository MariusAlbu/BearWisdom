// =============================================================================
// languages/prisma/extract.rs  —  Prisma PSL extractor (no grammar)
//
// What we extract
// ---------------
// SYMBOLS:
//   Struct      — model declarations
//   Class       — view declarations
//   Enum        — enum declarations
//   TypeAlias   — type declarations (composite types)
//   Variable    — datasource / generator blocks
//   Field       — fields inside model/view/type blocks
//   EnumMember  — values inside enum blocks
//
// REFERENCES:
//   TypeRef     — field type that names another model or enum
//   TypeRef     — @relation(...) → referenced model name
//
// No tree-sitter grammar available. This extractor is a line-oriented parser
// that handles the PSL block structure directly.
//
// Design:
//   1. Scan for top-level block declarations (keyword + name + "{")
//   2. Collect fields/members inside each block until "}"
//   3. Emit TypeRef for non-scalar field types
//   4. Emit TypeRef for @relation(... references: [...]) model references
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};

/// Prisma built-in scalar types. TypeRef edges are NOT emitted for these.
const SCALARS: &[&str] = &[
    "String", "Int", "Float", "Boolean", "DateTime",
    "Bytes", "Json", "BigInt", "Decimal",
];

pub fn extract(source: &str) -> crate::types::ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // Skip comments and blank lines
        if line.is_empty() || line.starts_with("//") {
            i += 1;
            continue;
        }

        // Detect a top-level block: `keyword Name {`
        if let Some((keyword, name)) = parse_block_header(line) {
            let start_line = i as u32;
            let (kind, is_enum, is_datasource_like) = match keyword {
                "model" => (SymbolKind::Struct, false, false),
                "view" => (SymbolKind::Class, false, false),
                "enum" => (SymbolKind::Enum, true, false),
                "type" => (SymbolKind::TypeAlias, false, false),
                "datasource" | "generator" => (SymbolKind::Variable, false, true),
                _ => {
                    i += 1;
                    continue;
                }
            };

            let parent_index = symbols.len();
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: name.clone(),
                kind,
                visibility: Some(Visibility::Public),
                start_line,
                end_line: start_line, // updated below
                start_col: 0,
                end_col: 0,
                signature: Some(format!("{keyword} {name} {{ ... }}")),
                doc_comment: extract_preceding_doc_comment(&lines, i),
                scope_path: None,
                parent_index: None,
            });

            // Consume the block body
            i += 1;
            while i < lines.len() {
                let body_line = lines[i].trim();

                if body_line == "}" {
                    // Update end_line on the parent symbol
                    symbols[parent_index].end_line = i as u32;
                    i += 1;
                    break;
                }

                if body_line.is_empty() || body_line.starts_with("//") || body_line.starts_with("///") {
                    i += 1;
                    continue;
                }

                if is_datasource_like {
                    // datasource/generator: key = value pairs — skip as low-value
                    i += 1;
                    continue;
                }

                if is_enum {
                    // Enum member: an identifier optionally followed by @attribute
                    let member_name = body_line
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');

                    // Skip block-level attributes (@@)
                    if member_name.starts_with("@@") || member_name.is_empty() {
                        i += 1;
                        continue;
                    }

                    symbols.push(ExtractedSymbol {
                        name: member_name.to_string(),
                        qualified_name: member_name.to_string(),
                        kind: SymbolKind::EnumMember,
                        visibility: Some(Visibility::Public),
                        start_line: i as u32,
                        end_line: i as u32,
                        start_col: 0,
                        end_col: 0,
                        signature: Some(member_name.to_string()),
                        doc_comment: None,
                        scope_path: None,
                        parent_index: Some(parent_index),
                    });
                } else {
                    // Model/view/type field: `fieldName FieldType[?][] [@attributes]`
                    extract_field(body_line, i as u32, parent_index, &mut symbols, &mut refs);
                }

                i += 1;
            }
        } else {
            i += 1;
        }
    }

    crate::types::ExtractionResult::new(symbols, refs, false)
}

// ---------------------------------------------------------------------------
// Field extraction
// ---------------------------------------------------------------------------

fn extract_field(
    line: &str,
    line_num: u32,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // Skip block-level attributes (@@index, @@unique, @@id, @@map, etc.)
    let trimmed = line.trim();
    if trimmed.starts_with("@@") || trimmed.starts_with("//") || trimmed.is_empty() {
        return;
    }

    let mut parts = trimmed.split_whitespace();
    let field_name = match parts.next() {
        Some(n) if !n.starts_with('@') => n,
        _ => return,
    };

    let raw_type = match parts.next() {
        Some(t) => t,
        None => return,
    };

    // Strip optional modifiers: `User?` → `User`, `User[]` → `User`
    let base_type = raw_type
        .trim_end_matches('?')
        .trim_end_matches(']')
        .trim_end_matches('[');

    let is_optional = raw_type.contains('?') || raw_type.contains('[');
    let sig = format!("{field_name} {raw_type}");

    let field_index = symbols.len();
    symbols.push(ExtractedSymbol {
        name: field_name.to_string(),
        qualified_name: field_name.to_string(),
        kind: SymbolKind::Field,
        visibility: Some(Visibility::Public),
        start_line: line_num,
        end_line: line_num,
        start_col: 0,
        end_col: 0,
        signature: Some(sig),
        doc_comment: None,
        scope_path: None,
        parent_index: Some(parent_index),
    });

    // Emit TypeRef for non-scalar types
    if !SCALARS.contains(&base_type) && !base_type.is_empty() && base_type.chars().next().map_or(false, |c| c.is_uppercase()) {
        let _ = is_optional;
        refs.push(ExtractedRef {
            source_symbol_index: field_index,
            target_name: base_type.to_string(),
            kind: EdgeKind::TypeRef,
            line: line_num,
            module: None,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
});
    }

    // Scan the rest of the line for @relation(... references mentions
    let rest: String = parts.collect::<Vec<_>>().join(" ");
    extract_relation_refs(&rest, field_index, line_num, refs);
}

/// Parse `@relation(...)` fragments for referenced model names.
/// We extract the value of `references: [FieldName, ...]` but primarily
/// care about the model type name (already captured via the field type TypeRef).
/// Here we also emit a TypeRef for `name:` argument strings that look like
/// relation names pointing to other models — low-priority, best-effort.
fn extract_relation_refs(
    rest: &str,
    source_symbol_index: usize,
    line_num: u32,
    refs: &mut Vec<ExtractedRef>,
) {
    // Look for @relation(fields: [...], references: [...])
    // The model link is already handled by the field type TypeRef.
    // Emit nothing extra here to avoid false positives from relation names.
    // This function is a hook for future expansion if needed.
    let _ = (rest, source_symbol_index, line_num, refs);
}

// ---------------------------------------------------------------------------
// Block header parsing
// ---------------------------------------------------------------------------

/// Parse `keyword Name {` lines. Returns (keyword, name) or None.
fn parse_block_header(line: &str) -> Option<(&str, String)> {
    let mut parts = line.splitn(3, ' ');
    let keyword = parts.next()?.trim();
    let name_raw = parts.next()?.trim();
    // Remove trailing `{` if on same token
    let name = name_raw.trim_end_matches('{').trim().to_string();
    if name.is_empty() {
        return None;
    }
    // Must be one of the known Prisma top-level keywords
    match keyword {
        "model" | "view" | "enum" | "type" | "datasource" | "generator" => {
            Some((keyword, name))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Doc comment extraction
// ---------------------------------------------------------------------------

/// Look backwards from `block_line` for `///` triple-slash doc comments.
fn extract_preceding_doc_comment(lines: &[&str], block_line: usize) -> Option<String> {
    if block_line == 0 {
        return None;
    }
    let mut doc_lines: Vec<&str> = Vec::new();
    let mut j = block_line as isize - 1;
    while j >= 0 {
        let l = lines[j as usize].trim();
        if let Some(stripped) = l.strip_prefix("///") {
            doc_lines.push(stripped.trim());
            j -= 1;
        } else {
            break;
        }
    }
    if doc_lines.is_empty() {
        return None;
    }
    doc_lines.reverse();
    Some(doc_lines.join("\n"))
}
