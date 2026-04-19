//! Yesod Shakespearean template plugins: Hamlet, Cassius, Lucius,
//! Julius. Each is minimal — file-stem host symbol, with Julius
//! dispatching its body as JavaScript (a reasonable approximation
//! since Julius is essentially JS with `#{}` interpolation).

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedOrigin, EmbeddedRegion, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

fn host_symbol(file_path: &str) -> ExtractionResult {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    let stem = std::path::Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string();
    let symbols = vec![ExtractedSymbol {
        name: stem.clone(), qualified_name: stem,
        kind: SymbolKind::Class, visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None,
    }];
    ExtractionResult { symbols, refs: Vec::new(), routes: Vec::new(), db_sets: Vec::new(), has_errors: false,
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

pub struct HamletPlugin;
impl LanguagePlugin for HamletPlugin {
    fn id(&self) -> &str { "hamlet" }
    fn language_ids(&self) -> &[&str] { &["hamlet"] }
    fn extensions(&self) -> &[&str] { &[".hamlet", ".shamlet", ".whamlet"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, _s: &str, p: &str, _l: &str) -> ExtractionResult { host_symbol(p) }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
}

pub struct CassiusPlugin;
impl LanguagePlugin for CassiusPlugin {
    fn id(&self) -> &str { "cassius" }
    fn language_ids(&self) -> &[&str] { &["cassius"] }
    fn extensions(&self) -> &[&str] { &[".cassius"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, _s: &str, p: &str, _l: &str) -> ExtractionResult { host_symbol(p) }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
}

pub struct LuciusPlugin;
impl LanguagePlugin for LuciusPlugin {
    fn id(&self) -> &str { "lucius" }
    fn language_ids(&self) -> &[&str] { &["lucius"] }
    fn extensions(&self) -> &[&str] { &[".lucius"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, _s: &str, p: &str, _l: &str) -> ExtractionResult { host_symbol(p) }
    fn embedded_regions(&self, source: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        vec![EmbeddedRegion {
            language_id: "css".into(),
            text: source.to_string(),
            line_offset: 0, col_offset: 0,
            origin: EmbeddedOrigin::StyleBlock,
            holes: Vec::new(), strip_scope_prefix: None,
        }]
    }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
}

pub struct JuliusPlugin;
impl LanguagePlugin for JuliusPlugin {
    fn id(&self) -> &str { "julius" }
    fn language_ids(&self) -> &[&str] { &["julius"] }
    fn extensions(&self) -> &[&str] { &[".julius"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, _s: &str, p: &str, _l: &str) -> ExtractionResult { host_symbol(p) }
    fn embedded_regions(&self, source: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        // The whole file is JS with `#{expr}` Haskell interpolation.
        // Leave holes empty for MVP — the JS parser treats `#{…}` as
        // invalid syntax, so errors may surface in highly
        // interpolated files. A future enhancement is to blank the
        // interpolation spans via the holes vector.
        vec![EmbeddedRegion {
            language_id: "javascript".into(),
            text: source.to_string(),
            line_offset: 0, col_offset: 0,
            origin: EmbeddedOrigin::ScriptBlock,
            holes: Vec::new(), strip_scope_prefix: None,
        }]
    }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
}
