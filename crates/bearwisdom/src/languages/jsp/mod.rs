//! JSP (.jsp, .jspx, .tag) — Java Server Pages.
//! `<% %>` / `<%= %>` → Java regions. `<%@ include file="…" %>` →
//! Imports ref.

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{
    EdgeKind, EmbeddedOrigin, EmbeddedRegion, ExtractedRef, ExtractedSymbol, ExtractionResult,
    SymbolKind, Visibility,
};

pub struct JspPlugin;

impl LanguagePlugin for JspPlugin {
    fn id(&self) -> &str { "jsp" }
    fn language_ids(&self) -> &[&str] { &["jsp"] }
    fn extensions(&self) -> &[&str] { &[".jsp", ".jspx", ".tag"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, source: &str, file_path: &str, _l: &str) -> ExtractionResult {
        let norm = file_path.replace('\\', "/");
        let name = norm.rsplit('/').next().unwrap_or(&norm);
        let stem = std::path::Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string();
        let symbols = vec![ExtractedSymbol {
            name: stem.clone(), qualified_name: stem,
            kind: SymbolKind::Class, visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: None, doc_comment: None, scope_path: None, parent_index: None,
        }];
        let mut refs: Vec<ExtractedRef> = Vec::new();
        for (line_no, line) in source.lines().enumerate() {
            let t = line.trim_start();
            // <%@ include file="x" %>
            if let Some(rest) = t.strip_prefix("<%@") {
                if let Some(pos) = rest.find("file=\"") {
                    let start = pos + 6;
                    if let Some(end) = rest[start..].find('"') {
                        let file = &rest[start..start + end];
                        let target = std::path::Path::new(file).file_stem().and_then(|s| s.to_str()).unwrap_or(file).to_string();
                        refs.push(ExtractedRef {
                            source_symbol_index: 0,
                            target_name: target,
                            kind: EdgeKind::Imports,
                            line: line_no as u32, module: None, chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
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
    fn embedded_regions(&self, source: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        let mut regions = crate::languages::common::extract_html_script_style_regions(source);
        let bytes = source.as_bytes();
        let mut i = 0usize;
        while i + 1 < bytes.len() {
            if bytes[i] == b'<' && bytes[i + 1] == b'%' {
                let kind = bytes.get(i + 2).copied();
                if kind == Some(b'@') || kind == Some(b'-') { i += 2; continue; }
                let is_expr = kind == Some(b'=');
                let body_start = if is_expr { i + 3 } else { i + 2 };
                let Some(close) = find_close(bytes, body_start) else { i += 2; continue; };
                if let Some(body) = source.get(body_start..close) {
                    let t = body.trim();
                    if !t.is_empty() {
                        let (line, col) = lc(bytes, body_start);
                        regions.push(EmbeddedRegion {
                            language_id: "java".into(),
                            text: if is_expr {
                                format!("class __Jsp {{ Object f() {{ return ({t}); }} }}\n")
                            } else {
                                format!("class __Jsp {{ void f() {{ {t} }} }}\n")
                            },
                            line_offset: line, col_offset: col,
                            origin: EmbeddedOrigin::TemplateExpr,
                            holes: Vec::new(), strip_scope_prefix: None,
                        });
                    }
                }
                i = close + 2; continue;
            }
            i += 1;
        }
        regions
    }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
}

fn find_close(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'%' && bytes[i + 1] == b'>' { return Some(i); }
        i += 1;
    }
    None
}
fn lc(bytes: &[u8], pos: usize) -> (u32, u32) {
    let mut line: u32 = 0; let mut nl: usize = 0;
    for (i, b) in bytes.iter().enumerate().take(pos) { if *b == b'\n' { line += 1; nl = i + 1; } }
    (line, (pos - nl) as u32)
}
