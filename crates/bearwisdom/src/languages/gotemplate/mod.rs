//! Go `text/template` / `html/template` plugin (`.tmpl`, `.gotmpl`,
//! `.gohtml`, `.tpl`).
//!
//! Recognizes:
//!   * `{{ expr }}`, `{{ .Field }}`, `{{ funcCall arg }}` → Go expression
//!   * `{{ define "name" }}...{{ end }}`                    → Field symbol
//!   * `{{ template "name" . }}`                            → Imports ref
//!   * `{{ range ... }}...{{ end }}` / `{{ if ... }}`        → directives

pub mod extract;
pub mod embedded;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct GoTemplatePlugin;

impl LanguagePlugin for GoTemplatePlugin {
    fn id(&self) -> &str { "gotemplate" }
    fn language_ids(&self) -> &[&str] { &["gotemplate"] }
    fn extensions(&self) -> &[&str] { &[".tmpl", ".gotmpl", ".gohtml", ".tpl"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> { None }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, s: &str, p: &str, _l: &str) -> ExtractionResult {
        extract::extract(s, p)
    }
    fn embedded_regions(&self, s: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(s)
    }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
}
