//! Apache Velocity (.vm, .vtl). `${var}` → Java expression.
//! `#parse("file.vm")`, `#include("file.vm")` → Imports ref.
//! `#macro(name)` → Field symbol.

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{
    EdgeKind, EmbeddedOrigin, EmbeddedRegion, ExtractedRef, ExtractedSymbol, ExtractionResult,
    SymbolKind, Visibility,
};

pub struct VelocityPlugin;

impl LanguagePlugin for VelocityPlugin {
    fn id(&self) -> &str { "velocity" }
    fn language_ids(&self) -> &[&str] { &["velocity"] }
    fn extensions(&self) -> &[&str] { &[".vm", ".vtl"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, source: &str, file_path: &str, _l: &str) -> ExtractionResult {
        let norm = file_path.replace('\\', "/");
        let name = norm.rsplit('/').next().unwrap_or(&norm);
        let stem = std::path::Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string();
        let mut symbols = vec![ExtractedSymbol {
            name: stem.clone(), qualified_name: stem.clone(),
            kind: SymbolKind::Class, visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: None, doc_comment: None, scope_path: None, parent_index: None,
        }];
        let mut refs: Vec<ExtractedRef> = Vec::new();
        for (line_no, line) in source.lines().enumerate() {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("#macro(") {
                let name: String = rest.chars().take_while(|c| *c != ' ' && *c != ',' && *c != ')').collect();
                if !name.is_empty() {
                    symbols.push(ExtractedSymbol {
                        name: name.clone(),
                        qualified_name: format!("{stem}.{name}"),
                        kind: SymbolKind::Field, visibility: Some(Visibility::Public),
                        start_line: line_no as u32, end_line: line_no as u32,
                        start_col: 0, end_col: 0,
                        signature: Some(t.into()), doc_comment: None,
                        scope_path: Some(stem.clone()), parent_index: Some(0),
                    });
                }
            }
            for kw in &["#parse(", "#include("] {
                if let Some(rest) = t.strip_prefix(kw) {
                    if let Some(s) = rest.strip_prefix('"') {
                        if let Some(e) = s.find('"') {
                            let file = &s[..e];
                            let target = std::path::Path::new(file).file_stem().and_then(|s| s.to_str()).unwrap_or(file).to_string();
                            refs.push(ExtractedRef {
                                source_symbol_index: 0, target_name: target,
                                kind: EdgeKind::Imports,
                                line: line_no as u32, module: None, chain: None,
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
        }
    }
    fn embedded_regions(&self, source: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        let mut regions = Vec::new();
        let bytes = source.as_bytes();
        let mut i = 0usize;
        while i + 1 < bytes.len() {
            // ${var} — simplest interpolation.
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
                                text: format!("class __Vm {{ Object f() {{ return ({t}); }} }}\n"),
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

fn lc(bytes: &[u8], pos: usize) -> (u32, u32) {
    let mut line: u32 = 0; let mut nl: usize = 0;
    for (i, b) in bytes.iter().enumerate().take(pos) { if *b == b'\n' { line += 1; nl = i + 1; } }
    (line, (pos - nl) as u32)
}
