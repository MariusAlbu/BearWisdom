//! Pug host extraction — file-stem symbol, mixin definitions,
//! include/extends Imports refs.

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

    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        let line = line_no as u32;
        if let Some(rest) = trimmed.strip_prefix("mixin ") {
            let name = rest
                .split(|c: char| c == '(' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                let qname = format!("{file_name}.{name}");
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
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
        } else if let Some(rest) = trimmed.strip_prefix("include ") {
            let target = normalize_template_path(rest.trim());
            if !target.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: host_index,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
        } else if let Some(rest) = trimmed.strip_prefix("extends ") {
            let target = normalize_template_path(rest.trim());
            if !target.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: host_index,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
        }
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

fn normalize_template_path(raw: &str) -> String {
    // Strip leading ./, trailing comments, and the `.pug` / `.jade`
    // extension. Match the target's file stem so the resolver can
    // connect it to another Pug file's host symbol.
    let mut s = raw.split('#').next().unwrap_or(raw).trim().to_string();
    if let Some(rest) = s.strip_prefix("./") {
        s = rest.to_string();
    }
    if let Some(stem) = std::path::Path::new(&s).file_stem().and_then(|x| x.to_str()) {
        let parent = std::path::Path::new(&s)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("");
        if parent.is_empty() {
            stem.to_string()
        } else {
            format!("{}/{}", parent.replace('\\', "/"), stem)
        }
    } else {
        s
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
    fn mixin_becomes_symbol() {
        let src = "mixin card(title, body)\n  .card= title\n";
        let r = extract(src, "mixins.pug");
        assert!(r.symbols.iter().any(|s| s.name == "card"));
    }

    #[test]
    fn include_becomes_imports_ref() {
        let src = "doctype html\nhtml\n  include ./head.pug\n";
        let r = extract(src, "layout.pug");
        assert!(r.refs.iter().any(|r| r.target_name == "head"
            && r.kind == EdgeKind::Imports));
    }

    #[test]
    fn extends_becomes_imports_ref() {
        let src = "extends layout\nblock content\n  p Body\n";
        let r = extract(src, "page.pug");
        assert!(r.refs.iter().any(|r| r.target_name == "layout"));
    }
}
