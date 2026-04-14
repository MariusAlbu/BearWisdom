//! Razor (`.cshtml`, `.razor`) language plugin.
//!
//! Razor mixes C# code blocks with HTML and optional `<script>` blocks.
//! There is no tree-sitter grammar that parses full Razor syntax in this
//! workspace, so the plugin:
//!
//!   * Produces no host-level symbols from `extract()` — the host
//!     language is a thin shell.
//!   * Implements `embedded_regions()` with a hand-rolled region detector
//!     (`embedded.rs`) that splits the file into C# (`@{...}`,
//!     `@code{...}`, `@functions{...}`, `@(expr)`) and JavaScript /
//!     TypeScript (`<script>`) chunks.
//!   * Lets the indexer dispatch each region through the sub-language's
//!     plugin — so a `@{ var svc = new UserService(); }` block becomes
//!     a real `UserService` type ref after the C# extractor parses it.
//!
//! Registered with extension `.cshtml` (MVC views) and `.razor` (Blazor
//! components) so both Razor dialects go through the same pipeline.

pub mod embedded;
pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct RazorPlugin;

impl LanguagePlugin for RazorPlugin {
    fn id(&self) -> &str { "razor" }

    fn language_ids(&self) -> &[&str] { &["razor"] }

    fn extensions(&self) -> &[&str] { &[".cshtml", ".razor"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        // No native Razor grammar. The host file is never parsed by a
        // grammar — all content flows through the embedded pipeline.
        None
    }

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
