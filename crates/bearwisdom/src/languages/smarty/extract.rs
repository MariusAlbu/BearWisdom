use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    let stem = std::path::Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string();
    let symbols = vec![ExtractedSymbol {
        name: stem.clone(), qualified_name: stem.clone(),
        kind: SymbolKind::Class, visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None,
    }];
    let mut refs: Vec<ExtractedRef> = Vec::new();
    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        for tag in &["{include ", "{extends "] {
            if let Some(rest) = trimmed.strip_prefix(tag) {
                if let Some(eq_pos) = rest.find("file=\"") {
                    let start = eq_pos + 6;
                    if let Some(end_rel) = rest[start..].find('"') {
                        let file = &rest[start..start + end_rel];
                        let p = std::path::Path::new(file);
                        let target = p.file_stem().and_then(|s| s.to_str()).unwrap_or(file).to_string();
                        refs.push(ExtractedRef {
                            source_symbol_index: 0,
                            target_name: target,
                            kind: EdgeKind::Imports,
                            line: line_no as u32,
                            module: None, chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
                    }
                }
            }
        }
    }
    ExtractionResult { symbols, refs, routes: Vec::new(), db_sets: Vec::new(), has_errors: false,
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
    }
}
