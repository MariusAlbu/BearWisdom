//! Thymeleaf templates (.th.html explicit form). `th:*` attribute
//! expressions dispatch to Java.
//!
//! Ordinary `.html` files under Spring `templates/` directories
//! are handled by the HTML plugin today — path-based routing to
//! Thymeleaf is a future enhancement.

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedOrigin, EmbeddedRegion, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

pub struct ThymeleafPlugin;

impl LanguagePlugin for ThymeleafPlugin {
    fn id(&self) -> &str { "thymeleaf" }
    fn language_ids(&self) -> &[&str] { &["thymeleaf"] }
    fn extensions(&self) -> &[&str] { &[".th.html"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, _s: &str, p: &str, _l: &str) -> ExtractionResult {
        let norm = p.replace('\\', "/");
        let name = norm.rsplit('/').next().unwrap_or(&norm);
        let stem = name.strip_suffix(".th.html").unwrap_or(name).to_string();
        let symbols = vec![ExtractedSymbol {
            name: stem.clone(), qualified_name: stem,
            kind: SymbolKind::Class, visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: None, doc_comment: None, scope_path: None, parent_index: None,
        }];
        ExtractionResult { symbols, refs: Vec::new(), routes: Vec::new(), db_sets: Vec::new(), has_errors: false,
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
        }
    }
    fn embedded_regions(&self, source: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        let mut regions = crate::languages::common::extract_html_script_style_regions(source);
        let bytes = source.as_bytes();
        // Scan for `th:xxx="${expr}"` or `th:xxx="@{expr}"` bindings.
        let mut i = 0usize;
        while i + 3 < bytes.len() {
            if &bytes[i..i + 3] == b"th:" {
                // Skip until `="`
                let mut j = i + 3;
                while j < bytes.len() && bytes[j] != b'"' && bytes[j] != b'>' { j += 1; }
                if j >= bytes.len() || bytes[j] != b'"' { i += 3; continue; }
                let val_start = j + 1;
                let mut k = val_start;
                while k < bytes.len() && bytes[k] != b'"' { k += 1; }
                if k >= bytes.len() { break; }
                if let Some(body) = source.get(val_start..k) {
                    // Strip ${…} or @{…} wrapper.
                    let t = body.trim();
                    let inner = if (t.starts_with("${") || t.starts_with("@{")) && t.ends_with('}') {
                        &t[2..t.len() - 1]
                    } else { t };
                    if !inner.is_empty() {
                        let (line, col) = lc(bytes, val_start);
                        regions.push(EmbeddedRegion {
                            language_id: "java".into(),
                            text: format!("class __Th {{ Object f() {{ return ({inner}); }} }}\n"),
                            line_offset: line, col_offset: col,
                            origin: EmbeddedOrigin::TemplateExpr,
                            holes: Vec::new(), strip_scope_prefix: None,
                        });
                    }
                }
                i = k + 1; continue;
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
