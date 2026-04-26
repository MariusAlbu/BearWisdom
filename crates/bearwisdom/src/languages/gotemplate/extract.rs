//! Go template host extraction — file-stem symbol, `{{define "name"}}`
//! blocks as symbols, `{{template "name"}}` as Imports refs.

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

    let bytes = source.as_bytes();
    let mut line: u32 = 0;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' { line += 1; i += 1; continue; }
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let body_start = i + 2;
            let Some(close) = find_double_close(bytes, body_start) else { i += 2; continue; };
            if let Some(body) = source.get(body_start..close) {
                let t = body.trim().trim_start_matches('-').trim_end_matches('-').trim();
                // {{define "name"}}
                if let Some(rest) = t.strip_prefix("define ") {
                    if let Some(name) = quoted(rest.trim()) {
                        symbols.push(ExtractedSymbol {
                            name: name.clone(),
                            qualified_name: format!("{stem}.{name}"),
                            kind: SymbolKind::Field,
                            visibility: Some(Visibility::Public),
                            start_line: line, end_line: line, start_col: 0, end_col: 0,
                            signature: Some(t.to_string()),
                            doc_comment: None,
                            scope_path: Some(stem.clone()),
                            parent_index: Some(host_index),
                        });
                    }
                } else if let Some(rest) = t.strip_prefix("template ") {
                    // {{template "name" .}}  →  Imports ref to the named template.
                    let tok = rest.trim().split_whitespace().next().unwrap_or("");
                    if let Some(name) = quoted(tok) {
                        refs.push(ExtractedRef {
                            source_symbol_index: host_index,
                            target_name: name,
                            kind: EdgeKind::Imports,
                            line, module: None, chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
                    }
                }
            }
            i = close + 2;
            continue;
        }
        i += 1;
    }
    ExtractionResult { symbols, refs, routes: Vec::new(), db_sets: Vec::new(), has_errors: false,
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

fn quoted(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() >= 2 && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('`') && s.ends_with('`'))) {
        Some(s[1..s.len() - 1].to_string())
    } else { None }
}

fn find_double_close(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'}' && bytes[i + 1] == b'}' { return Some(i); }
        i += 1;
    }
    None
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
    fn define_becomes_symbol() {
        let src = r#"{{define "header"}}<h1>Hi</h1>{{end}}"#;
        let r = extract(src, "views.tmpl");
        assert!(r.symbols.iter().any(|s| s.name == "header"));
    }

    #[test]
    fn template_becomes_imports_ref() {
        let src = r#"{{template "footer" .}}"#;
        let r = extract(src, "page.tmpl");
        assert!(r.refs.iter().any(|r| r.target_name == "footer"));
    }
}
