//! Polyglot Notebook host-level extraction.
//!
//! Emits a file-level `Class` symbol named after the notebook stem
//! plus one `Field` symbol per cell (named `<kernel>#<idx>`) so
//! `file_symbols` on a `.dib` shows a cell outline.

use super::cells;
use crate::types::{ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();

    let file_name = file_stem(file_path);
    symbols.push(ExtractedSymbol {
        name: file_name.clone(),
        qualified_name: file_name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
    let host_index: usize = 0;

    for (idx, cell) in cells::parse_cells(source).into_iter().enumerate() {
        let anchor = format!("{}#{}", cell.kernel, idx);
        let end_line = cell.body_line_offset + cell.body.matches('\n').count() as u32;
        symbols.push(ExtractedSymbol {
            name: anchor.clone(),
            qualified_name: format!("{file_name}.{anchor}"),
            kind: SymbolKind::Field,
            visibility: Some(Visibility::Public),
            start_line: cell.body_line_offset.saturating_sub(1),
            end_line,
            start_col: 0,
            end_col: 0,
            signature: Some(cell.kernel.clone()),
            doc_comment: None,
            scope_path: Some(file_name.clone()),
            parent_index: Some(host_index),
        });
    }

    ExtractionResult {
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: false,
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
    }
}

fn file_stem(file_path: &str) -> String {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    stem.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_file_host_symbol_and_cell_anchors() {
        let src = "#!csharp\nvar x = 1;\n\n#!fsharp\nlet y = 2\n";
        let r = extract(src, "notebooks/demo.dib");
        assert_eq!(r.symbols[0].name, "demo");
        let anchors: Vec<&str> = r
            .symbols
            .iter()
            .skip(1)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(anchors, vec!["csharp#0", "fsharp#1"]);
    }

    #[test]
    fn no_cells_still_emits_host() {
        let r = extract("", "empty.dib");
        assert_eq!(r.symbols.len(), 1);
    }
}
