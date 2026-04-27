//! EJS host extraction — just the file-stem symbol (directive symbols
//! aren't meaningful in EJS, which is pure JS embedded in HTML).

use crate::types::{ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

pub fn extract(_source: &str, file_path: &str) -> ExtractionResult {
    let file_name = file_stem(file_path);
    let symbols = vec![ExtractedSymbol {
        name: file_name.clone(),
        qualified_name: file_name,
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
    }];
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
    std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string()
}
