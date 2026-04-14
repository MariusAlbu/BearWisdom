//! Twig (`.twig`, `.html.twig`) language plugin.
//!
//! Twig is the Symfony / Drupal templating language. The plugin:
//!
//!   * Has no native tree-sitter grammar — `grammar()` returns `None`.
//!   * Emits ONE file-level `Class` symbol named after the dotted
//!     template path (`templates/users/show.html.twig` → `users.show`),
//!     plus `Method` per `{% block %}`, `Function` per `{% macro %}`,
//!     and an `Imports` ref per template-relating directive
//!     (`extends`, `include`, `use`, `import`, `from`, `embed`).
//!   * Routes `<script>` and `<style>` blocks through the JS / TS / CSS
//!     / SCSS sub-extractors. Twig expressions (`{{ expr }}`) and
//!     directive bodies are NOT sub-extracted — there's no Twig
//!     expression grammar in the workspace and the host extractor
//!     already surfaces every queryable construct.

pub mod embedded;
pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct TwigPlugin;

impl LanguagePlugin for TwigPlugin {
    fn id(&self) -> &str { "twig" }

    fn language_ids(&self) -> &[&str] { &["twig"] }

    fn extensions(&self) -> &[&str] { &[".twig", ".html.twig"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> { None }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source, file_path)
    }

    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] { &[] }
    fn ref_node_kinds(&self) -> &[&str] { &[] }
}
