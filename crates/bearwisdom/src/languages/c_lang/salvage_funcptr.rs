// =============================================================================
// c_lang/salvage_funcptr.rs  —  recover function-pointer declarations
// =============================================================================

use std::collections::HashSet;

use crate::types::{ExtractedSymbol, SymbolKind};

/// Scan source for function-pointer-table declarations of the shape
/// `<prefix> <return_type> (*NAME)(<params>) <suffix>;` and emit a Function
/// symbol for any NAME not already in the symbols table.
///
/// The `(*NAME)(` token sequence is unambiguous at file scope — it appears
/// only in function-pointer declarations and (rarely) inside parameter
/// lists. Restricting to lines that end in `;` filters most false hits.
/// Anything tree-sitter parsed correctly will already have a symbol with
/// the same name, so the dedup check makes us a no-op there.
pub(super) fn salvage_missed_function_pointer_decls(source: &str, symbols: &mut Vec<ExtractedSymbol>) {
    let mut existing: HashSet<String> =
        symbols.iter().map(|s| s.name.clone()).collect();

    for (line_idx, line) in source.lines().enumerate() {
        let trimmed = line.trim_end();
        // Require trailing `;` — declaration, not call site.
        if !trimmed.ends_with(';') {
            continue;
        }
        // Look for the `(*NAME)(` shape.
        let Some(name) = scan_funptr_decl_name(trimmed) else { continue };
        if existing.contains(name) {
            continue;
        }
        let line_no = line_idx as u32;
        symbols.push(ExtractedSymbol {
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind: SymbolKind::Function,
            visibility: None,
            start_line: line_no,
            end_line: line_no,
            start_col: 0,
            end_col: name.len() as u32,
            signature: Some(trimmed.trim_start().to_string()),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
        existing.insert(name.to_string());
    }
}

/// Find `(*NAME)(` in `line` and return NAME, or None if the shape is absent.
fn scan_funptr_decl_name(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 4 < bytes.len() {
        // Match `(`, optional whitespace, `*`, optional whitespace, identifier, `)`, `(`.
        if bytes[i] != b'(' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() { j += 1; }
        if j >= bytes.len() || bytes[j] != b'*' { i += 1; continue; }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() { j += 1; }
        let name_start = j;
        while j < bytes.len()
            && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_')
        { j += 1; }
        let name_end = j;
        if name_end == name_start {
            i += 1;
            continue;
        }
        // First char must be alpha or `_`.
        let first = bytes[name_start];
        if !(first.is_ascii_alphabetic() || first == b'_') {
            i += 1;
            continue;
        }
        while j < bytes.len() && bytes[j].is_ascii_whitespace() { j += 1; }
        if j >= bytes.len() || bytes[j] != b')' { i += 1; continue; }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() { j += 1; }
        if j >= bytes.len() || bytes[j] != b'(' { i += 1; continue; }
        return Some(&line[name_start..name_end]);
    }
    None
}
