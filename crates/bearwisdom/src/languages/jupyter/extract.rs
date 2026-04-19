//! Jupyter host-level extraction.
//!
//! Emits a file-stem host symbol plus one cell-anchor symbol per
//! code cell so `file_symbols` on an `.ipynb` shows a cell outline.
//! Markdown cells don't emit anchors — keeping the outline clean.

use super::cell_scanner::{self, CellKind};
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
    let host_index = 0usize;

    let Some(nb) = cell_scanner::parse_notebook(source) else {
        return ExtractionResult {
            symbols,
            refs: Vec::new(),
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors: true,
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
        };
    };

    for (idx, cell) in nb.cells.iter().enumerate() {
        if cell.cell_type != CellKind::Code {
            continue;
        }
        let anchor = format!("{}#{}", nb.kernel_language, idx);
        let line_count = cell.body.matches('\n').count() as u32;
        symbols.push(ExtractedSymbol {
            name: anchor.clone(),
            qualified_name: format!("{file_name}.{anchor}"),
            kind: SymbolKind::Field,
            visibility: Some(Visibility::Public),
            start_line: cell.body_line_offset,
            end_line: cell.body_line_offset + line_count,
            start_col: 0,
            end_col: 0,
            signature: Some(nb.kernel_language.clone()),
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
    fn emits_cell_anchors_for_code_cells_only() {
        let src = r##"{
 "cells": [
  {"cell_type": "code", "source": "x = 1", "metadata": {}},
  {"cell_type": "markdown", "source": "# Hi", "metadata": {}},
  {"cell_type": "code", "source": "y = 2", "metadata": {}}
 ],
 "metadata": {"kernelspec": {"language": "python"}}
}"##;
        let r = extract(src, "nb.ipynb");
        let anchors: Vec<&str> = r
            .symbols
            .iter()
            .skip(1)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(anchors, vec!["python#0", "python#2"]);
    }

    #[test]
    fn malformed_notebook_still_emits_host_with_error_flag() {
        let r = extract("not json", "bad.ipynb");
        assert_eq!(r.symbols.len(), 1);
        assert!(r.has_errors);
    }
}
