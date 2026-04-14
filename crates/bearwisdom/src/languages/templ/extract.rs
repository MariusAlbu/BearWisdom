//! Templ host extraction — file-stem symbol, `templ Name(...)` →
//! Function symbol, `@Child(...)` → Calls ref.

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();
    let stem = file_stem(file_path);
    symbols.push(ExtractedSymbol {
        name: stem.clone(), qualified_name: stem.clone(),
        kind: SymbolKind::Class, visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None,
    });
    let host_index = 0usize;

    // Pass 1: find `templ Name(args)` declarations.
    let mut templ_indexes: Vec<usize> = Vec::new();
    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("templ ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                templ_indexes.push(symbols.len());
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name: format!("{stem}.{name}"),
                    kind: SymbolKind::Function,
                    visibility: Some(Visibility::Public),
                    start_line: line_no as u32,
                    end_line: line_no as u32,
                    start_col: 0, end_col: 0,
                    signature: Some(trimmed.to_string()),
                    doc_comment: None,
                    scope_path: Some(stem.clone()),
                    parent_index: Some(host_index),
                });
            }
        }
    }

    // Pass 2: find `@ComponentCall(...)` refs attributed to the
    // enclosing templ function when in scope, else the host.
    for (line_no, line) in source.lines().enumerate() {
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'@' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_uppercase() {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len()
                    && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'.')
                {
                    j += 1;
                }
                let name = line.get(start..j).unwrap_or("").to_string();
                if !name.is_empty() {
                    // Determine enclosing templ (rough: last templ defined before this line).
                    let src_idx = templ_indexes
                        .iter()
                        .rev()
                        .find(|&&idx| (symbols[idx].start_line as usize) <= line_no)
                        .copied()
                        .unwrap_or(host_index);
                    refs.push(ExtractedRef {
                        source_symbol_index: src_idx,
                        target_name: name,
                        kind: EdgeKind::Calls,
                        line: line_no as u32,
                        module: None, chain: None,
                    });
                }
                i = j;
                continue;
            }
            i += 1;
        }
    }

    ExtractionResult { symbols, refs, routes: Vec::new(), db_sets: Vec::new(), has_errors: false }
}

fn file_stem(file_path: &str) -> String {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    std::path::Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templ_declaration_becomes_function_symbol() {
        let src = "package views\n\ntempl UserCard(user User) {\n  <div>Hi</div>\n}\n";
        let r = extract(src, "views.templ");
        assert!(r.symbols.iter().any(|s| s.name == "UserCard" && s.kind == SymbolKind::Function));
    }

    #[test]
    fn at_component_becomes_calls_ref() {
        let src = "templ Layout() {\n  @Header()\n  @Footer()\n}\n";
        let r = extract(src, "layout.templ");
        assert!(r.refs.iter().any(|r| r.target_name == "Header"));
        assert!(r.refs.iter().any(|r| r.target_name == "Footer"));
    }
}
