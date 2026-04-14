//! Nunjucks host extraction — file-stem symbol, `{% block %}` symbols,
//! `extends`/`include`/`import` Imports refs.

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

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

    let bytes = source.as_bytes();
    let mut line: u32 = 0;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'%' {
            let body_start = i + 2;
            let Some(close) = find_percent_close(bytes, body_start) else {
                i += 2;
                continue;
            };
            if let Some(body) = source.get(body_start..close) {
                let trimmed = body.trim().trim_start_matches('-').trim_end_matches('-').trim();
                if let Some(rest) = trimmed.strip_prefix("block ") {
                    let name = rest.split_whitespace().next().unwrap_or("").to_string();
                    if !name.is_empty() {
                        symbols.push(ExtractedSymbol {
                            name: name.clone(),
                            qualified_name: format!("{file_name}.{name}"),
                            kind: SymbolKind::Field,
                            visibility: Some(Visibility::Public),
                            start_line: line,
                            end_line: line,
                            start_col: 0,
                            end_col: 0,
                            signature: Some(trimmed.to_string()),
                            doc_comment: None,
                            scope_path: Some(file_name.clone()),
                            parent_index: Some(host_index),
                        });
                    }
                } else if let Some(rest) = trimmed.strip_prefix("extends ") {
                    if let Some(name) = strip_quotes(rest.trim()) {
                        refs.push(ExtractedRef {
                            source_symbol_index: host_index,
                            target_name: strip_extension(&name),
                            kind: EdgeKind::Imports,
                            line,
                            module: None,
                            chain: None,
                        });
                    }
                } else if let Some(rest) = trimmed.strip_prefix("include ") {
                    if let Some(name) = strip_quotes(rest.trim()) {
                        refs.push(ExtractedRef {
                            source_symbol_index: host_index,
                            target_name: strip_extension(&name),
                            kind: EdgeKind::Imports,
                            line,
                            module: None,
                            chain: None,
                        });
                    }
                } else if let Some(rest) = trimmed.strip_prefix("import ") {
                    let tok = rest.split_whitespace().next().unwrap_or("");
                    if let Some(name) = strip_quotes(tok.trim()) {
                        refs.push(ExtractedRef {
                            source_symbol_index: host_index,
                            target_name: strip_extension(&name),
                            kind: EdgeKind::Imports,
                            line,
                            module: None,
                            chain: None,
                        });
                    }
                }
            }
            i = close + 2;
            continue;
        }
        i += 1;
    }

    ExtractionResult {
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: false,
    }
}

fn find_percent_close(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'%' && bytes[i + 1] == b'}' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn strip_quotes(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() >= 2 && (s.starts_with('"') && s.ends_with('"') || s.starts_with('\'') && s.ends_with('\''))
    {
        Some(s[1..s.len() - 1].to_string())
    } else {
        None
    }
}

fn strip_extension(path: &str) -> String {
    let p = std::path::Path::new(path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(path);
    let parent = p.parent().and_then(|p| p.to_str()).unwrap_or("");
    if parent.is_empty() {
        stem.to_string()
    } else {
        format!("{}/{}", parent.replace('\\', "/"), stem)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_directive_becomes_symbol() {
        let src = "{% block content %}hi{% endblock %}";
        let r = extract(src, "page.njk");
        assert!(r.symbols.iter().any(|s| s.name == "content"));
    }

    #[test]
    fn extends_becomes_imports_ref() {
        let src = "{% extends \"base.njk\" %}\n{% block body %}x{% endblock %}\n";
        let r = extract(src, "page.njk");
        assert!(r.refs.iter().any(|r| r.target_name == "base"));
    }

    #[test]
    fn include_becomes_imports_ref() {
        let src = "{% include \"partials/header.njk\" %}";
        let r = extract(src, "layout.njk");
        assert!(r.refs.iter().any(|r| r.target_name == "partials/header"));
    }
}
