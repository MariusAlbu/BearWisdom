//! FreeMarker (`.ftl`, `.ftlh`). `${expr}` → Java expression,
//! `<#include "file">` → Imports ref, `<#macro name>` → Field symbol.

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{
    EdgeKind, EmbeddedOrigin, EmbeddedRegion, ExtractedRef, ExtractedSymbol, ExtractionResult,
    SymbolKind, Visibility,
};

pub struct FreemarkerPlugin;

impl LanguagePlugin for FreemarkerPlugin {
    fn id(&self) -> &str { "freemarker" }
    fn language_ids(&self) -> &[&str] { &["freemarker"] }
    fn extensions(&self) -> &[&str] { &[".ftl", ".ftlh", ".ftlx"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, source: &str, file_path: &str, _l: &str) -> ExtractionResult {
        let stem = stem(file_path);
        let mut symbols = vec![host(&stem)];
        let mut refs: Vec<ExtractedRef> = Vec::new();
        for (line_no, line) in source.lines().enumerate() {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("<#macro ") {
                let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if !name.is_empty() {
                    symbols.push(field(&stem, &name, line_no as u32, t));
                }
            } else if let Some(rest) = t.strip_prefix("<#include ") {
                if let Some(name) = quoted(rest) {
                    refs.push(imports_ref(&name, line_no as u32));
                }
            }
        }
        ExtractionResult { symbols, refs, routes: Vec::new(), db_sets: Vec::new(), has_errors: false }
    }
    fn embedded_regions(&self, source: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        let mut regions = crate::languages::common::extract_html_script_style_regions(source);
        let bytes = source.as_bytes();
        let mut i = 0usize;
        while i + 1 < bytes.len() {
            if bytes[i] == b'$' && bytes[i + 1] == b'{' {
                let start = i + 2;
                let mut d = 1; let mut j = start;
                while j < bytes.len() && d > 0 {
                    match bytes[j] { b'{' => d += 1, b'}' => d -= 1, _ => {} }
                    if d == 0 { break; }
                    j += 1;
                }
                if j < bytes.len() && d == 0 {
                    if let Some(t) = source.get(start..j) {
                        let t = t.trim();
                        if !t.is_empty() {
                            let (line, col) = lc(bytes, start);
                            regions.push(EmbeddedRegion {
                                language_id: "java".into(),
                                text: format!("class __Ft {{ Object f() {{ return ({t}); }} }}\n"),
                                line_offset: line, col_offset: col,
                                origin: EmbeddedOrigin::TemplateExpr,
                                holes: Vec::new(), strip_scope_prefix: None,
                            });
                        }
                    }
                    i = j + 1; continue;
                }
            }
            i += 1;
        }
        regions
    }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
}

fn stem(p: &str) -> String {
    let norm = p.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    std::path::Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string()
}
fn host(stem: &str) -> ExtractedSymbol {
    ExtractedSymbol { name: stem.into(), qualified_name: stem.into(),
        kind: SymbolKind::Class, visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None }
}
fn field(stem: &str, name: &str, line: u32, sig: &str) -> ExtractedSymbol {
    ExtractedSymbol { name: name.into(), qualified_name: format!("{stem}.{name}"),
        kind: SymbolKind::Field, visibility: Some(Visibility::Public),
        start_line: line, end_line: line, start_col: 0, end_col: 0,
        signature: Some(sig.into()), doc_comment: None,
        scope_path: Some(stem.into()), parent_index: Some(0) }
}
fn imports_ref(name: &str, line: u32) -> ExtractedRef {
    let p = std::path::Path::new(name);
    let target = p.file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string();
    ExtractedRef { source_symbol_index: 0, target_name: target,
        kind: EdgeKind::Imports, line, module: None, chain: None }
}
fn quoted(s: &str) -> Option<String> {
    let s = s.trim();
    if s.starts_with('"') {
        let rest = &s[1..];
        rest.find('"').map(|e| rest[..e].to_string())
    } else { None }
}
fn lc(bytes: &[u8], pos: usize) -> (u32, u32) {
    let mut line: u32 = 0; let mut nl: usize = 0;
    for (i, b) in bytes.iter().enumerate().take(pos) { if *b == b'\n' { line += 1; nl = i + 1; } }
    (line, (pos - nl) as u32)
}
