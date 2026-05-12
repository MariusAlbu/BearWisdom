//! YAML language plugin.
//!
//! The primary value of parsing YAML is to recover shell scripts embedded
//! in CI config files — GitHub Actions (`run:`), GitLab (`script:`,
//! `before_script:`, `after_script:`), Azure Pipelines (`script:`). The
//! file-path-based detector lives in [`embedded`].
//!
//! Symbol extraction for YAML is intentionally minimal: one file-scoped
//! class symbol plus one Field symbol per top-level mapping key. The
//! downstream index knows the file exists and what top-level keys it
//! defines, which is enough for cross-file dispatch without exposing
//! every nested sequence entry as a graph node.

pub mod ansible;
pub mod embedded;
pub mod extract;
pub mod resolve;

use std::sync::Arc;

use crate::indexer::resolve::engine::LanguageResolver;
use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct YamlPlugin;

impl LanguagePlugin for YamlPlugin {
    fn id(&self) -> &str { "yaml" }
    fn language_ids(&self) -> &[&str] { &["yaml"] }
    fn extensions(&self) -> &[&str] { &[".yml", ".yaml"] }
    fn grammar(&self, _l: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_yaml::LANGUAGE.into())
    }
    fn scope_kinds(&self) -> &[ScopeKind] { &[] }
    fn extract(&self, s: &str, p: &str, _l: &str) -> ExtractionResult {
        extract::extract(s, p)
    }
    fn embedded_regions(&self, s: &str, p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(s, p)
    }
    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
    fn resolver(&self) -> Option<Arc<dyn LanguageResolver>> {
        Some(Arc::new(resolve::YamlResolver))
    }
}
