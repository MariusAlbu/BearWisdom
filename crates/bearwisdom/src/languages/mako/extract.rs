use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    let stem = if let Some(x) = name.strip_suffix(".html.mako") { x.to_string() }
        else { std::path::Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string() };
    let mut symbols = vec![ExtractedSymbol {
        name: stem.clone(), qualified_name: stem.clone(),
        kind: SymbolKind::Class, visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None,
    }];
    let host_index = 0usize;
    let mut refs: Vec<ExtractedRef> = Vec::new();

    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        // <%def name="foo()">
        if let Some(rest) = trimmed.strip_prefix("<%def") {
            if let Some(name) = extract_attr(rest, "name") {
                let ident = name.split(|c: char| c == '(' || c.is_whitespace()).next().unwrap_or("").to_string();
                if !ident.is_empty() {
                    symbols.push(ExtractedSymbol {
                        name: ident.clone(),
                        qualified_name: format!("{stem}.{ident}"),
                        kind: SymbolKind::Field, visibility: Some(Visibility::Public),
                        start_line: line_no as u32, end_line: line_no as u32,
                        start_col: 0, end_col: 0,
                        signature: Some(trimmed.to_string()),
                        doc_comment: None,
                        scope_path: Some(stem.clone()),
                        parent_index: Some(host_index),
                    });
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("<%include") {
            if let Some(file) = extract_attr(rest, "file") {
                refs.push(ExtractedRef {
                    source_symbol_index: host_index,
                    target_name: strip_ext(&file),
                    kind: EdgeKind::Imports,
                    line: line_no as u32,
                    module: None, chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        } else if let Some(rest) = trimmed.strip_prefix("<%inherit") {
            if let Some(file) = extract_attr(rest, "file") {
                refs.push(ExtractedRef {
                    source_symbol_index: host_index,
                    target_name: strip_ext(&file),
                    kind: EdgeKind::Imports,
                    line: line_no as u32,
                    module: None, chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        }
    }

    ExtractionResult { symbols, refs, routes: Vec::new(), db_sets: Vec::new(), has_errors: false,
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
    }
}

fn extract_attr(s: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let idx = s.find(&needle)?;
    let start = idx + needle.len();
    let rest = s.get(start..)?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn strip_ext(path: &str) -> String {
    let p = std::path::Path::new(path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(path);
    let parent = p.parent().and_then(|p| p.to_str()).unwrap_or("");
    if parent.is_empty() { stem.to_string() } else { format!("{}/{}", parent.replace('\\', "/"), stem) }
}
