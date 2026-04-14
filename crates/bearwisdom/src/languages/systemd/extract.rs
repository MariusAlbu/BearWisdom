use crate::types::{ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    let stem = name.to_string();  // keep full name including extension for uniqueness
    let mut symbols = vec![ExtractedSymbol {
        name: stem.clone(), qualified_name: stem.clone(),
        kind: SymbolKind::Class, visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None,
    }];
    let mut current_section: Option<String> = None;
    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.is_empty() { continue; }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = Some(trimmed.trim_matches(|c| c == '[' || c == ']').to_string());
            continue;
        }
        if let Some(eq) = trimmed.find('=') {
            let key = trimmed[..eq].trim().to_string();
            if !key.is_empty() {
                let scope = current_section.clone().unwrap_or_else(|| stem.clone());
                symbols.push(ExtractedSymbol {
                    name: key.clone(),
                    qualified_name: format!("{}.{}.{}", stem, scope, key),
                    kind: SymbolKind::Field,
                    visibility: Some(Visibility::Public),
                    start_line: line_no as u32, end_line: line_no as u32,
                    start_col: 0, end_col: 0,
                    signature: None, doc_comment: None,
                    scope_path: Some(scope),
                    parent_index: Some(0),
                });
            }
        }
    }
    ExtractionResult { symbols, refs: Vec::new(), routes: Vec::new(), db_sets: Vec::new(), has_errors: false }
}
