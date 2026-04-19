use crate::types::{ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    let stem = std::path::Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string();
    let mut symbols = vec![ExtractedSymbol {
        name: stem.clone(), qualified_name: stem.clone(),
        kind: SymbolKind::Class, visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None,
    }];
    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        for kw in &["location ", "server ", "upstream "] {
            if let Some(rest) = trimmed.strip_prefix(kw) {
                let name_part: String = rest
                    .chars()
                    .take_while(|c| !matches!(c, '{' | '\n' | '\r'))
                    .collect::<String>()
                    .trim()
                    .to_string();
                let display = if name_part.is_empty() {
                    kw.trim_end().to_string()
                } else {
                    format!("{}:{}", kw.trim_end(), name_part)
                };
                symbols.push(ExtractedSymbol {
                    name: display.clone(),
                    qualified_name: format!("{stem}.{display}"),
                    kind: SymbolKind::Field,
                    visibility: Some(Visibility::Public),
                    start_line: line_no as u32, end_line: line_no as u32,
                    start_col: 0, end_col: 0,
                    signature: Some(trimmed.to_string()),
                    doc_comment: None,
                    scope_path: Some(stem.clone()),
                    parent_index: Some(0),
                });
            }
        }
    }
    ExtractionResult { symbols, refs: Vec::new(), routes: Vec::new(), db_sets: Vec::new(), has_errors: false,
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}
