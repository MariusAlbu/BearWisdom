// =============================================================================
// c_lang/salvage_defines.rs  —  recover #define macros that tree-sitter dropped
// =============================================================================

use std::collections::HashSet;

use crate::types::{ExtractedSymbol, SymbolKind};

/// Scan source for `#define IDENT [...] ` lines and emit symbols for any
/// name not already in the symbols table. Function-like macros (`#define
/// FOO(a, b) ...`) emit as Function; object-like (`#define FOO bar`) emit
/// as Variable.
///
/// Runs unconditionally — the dedup against existing names makes it a
/// no-op when tree-sitter already extracted everything. Cost: one
/// lines() pass over the source, O(N) substring matching.
pub(super) fn salvage_missed_defines(source: &str, symbols: &mut Vec<ExtractedSymbol>) {
    let mut existing: HashSet<String> =
        symbols.iter().map(|s| s.name.clone()).collect();

    for (line_idx, line) in source.lines().enumerate() {
        let stripped = line.trim_start();
        let after_hash = match stripped.strip_prefix('#') {
            Some(s) => s.trim_start(),
            None => continue,
        };
        let after_define = match after_hash.strip_prefix("define") {
            Some(s) => s,
            None => continue,
        };
        // Require whitespace after `define` so we don't accidentally
        // match `defined`, `defines`, etc.
        if !after_define.starts_with(|c: char| c.is_whitespace()) {
            continue;
        }
        let after_define = after_define.trim_start();
        // Parse the identifier.
        let mut iter = after_define.char_indices();
        let first = match iter.next() {
            Some((_, c)) if c.is_alphabetic() || c == '_' => c,
            _ => continue,
        };
        let mut end = first.len_utf8();
        for (idx, c) in iter {
            if c.is_alphanumeric() || c == '_' {
                end = idx + c.len_utf8();
            } else {
                break;
            }
        }
        let name = &after_define[..end];
        if existing.contains(name) {
            continue;
        }
        let is_function = after_define[end..].starts_with('(');
        let kind = if is_function {
            SymbolKind::Function
        } else {
            SymbolKind::Variable
        };
        let line_no = line_idx as u32;
        symbols.push(ExtractedSymbol {
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind,
            visibility: None,
            start_line: line_no,
            end_line: line_no,
            start_col: 0,
            end_col: end as u32,
            signature: Some(format!("#define {name}")),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        });
        existing.insert(name.to_string());
    }
}
