//! Handlebars host-level extraction — file-stem symbol, block symbols,
//! partial-include refs.

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
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // Parse the mustache tag up to the matching `}}` (or `}}}` for raw).
            let expr_start = i + 2;
            let triple = bytes.get(i + 2).copied() == Some(b'{');
            let expr_body_start = if triple { expr_start + 1 } else { expr_start };
            let Some(rel_end) = find_close(&bytes[expr_body_start..], triple) else {
                i += 2;
                continue;
            };
            let expr_end = expr_body_start + rel_end;
            if let Some(body) = source.get(expr_body_start..expr_end) {
                let trimmed = body.trim();
                if let Some(rest) = trimmed.strip_prefix('#') {
                    // `#each xs`, `#if cond` — block open. Symbol name is the
                    // helper (`each`, `if`).
                    let helper = rest.split_whitespace().next().unwrap_or("").to_string();
                    if !helper.is_empty() {
                        let qname = format!("{file_name}.{helper}");
                        symbols.push(ExtractedSymbol {
                            name: helper,
                            qualified_name: qname,
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
                } else if let Some(rest) = trimmed.strip_prefix('>') {
                    // Partial include `{{> partial-name args}}`.
                    let name = rest.trim().split_whitespace().next().unwrap_or("").to_string();
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index: host_index,
                            target_name: name,
                            kind: EdgeKind::Imports,
                            line,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
                    }
                }
            }
            i = expr_end + if triple { 3 } else { 2 };
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
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

/// Find `}}` (or `}}}` for triple-brace raw output) from start of `bytes`.
/// Returns the offset of the first `}` of the closer.
fn find_close(bytes: &[u8], triple: bool) -> Option<usize> {
    let needed = if triple { 3 } else { 2 };
    let mut i = 0;
    while i + needed <= bytes.len() {
        if bytes[i] == b'}'
            && bytes[i + 1] == b'}'
            && (!triple || bytes[i + 2] == b'}')
        {
            return Some(i);
        }
        i += 1;
    }
    None
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
    fn each_block_becomes_symbol() {
        let src = "{{#each items}}<li>{{name}}</li>{{/each}}";
        let r = extract(src, "list.hbs");
        assert!(r.symbols.iter().any(|s| s.name == "each"));
    }

    #[test]
    fn partial_include_becomes_ref() {
        let src = "{{> header}}\n<main>body</main>\n";
        let r = extract(src, "layout.hbs");
        assert!(r.refs.iter().any(|r| r.target_name == "header"));
    }

    #[test]
    fn if_block_becomes_symbol() {
        let src = "{{#if isLoggedIn}}welcome{{/if}}";
        let r = extract(src, "header.hbs");
        assert!(r.symbols.iter().any(|s| s.name == "if"));
    }
}
