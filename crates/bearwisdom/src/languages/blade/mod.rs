//! Blade (`.blade.php`) language plugin.
//!
//! Blade is Laravel's templating language layered on top of PHP. The
//! plugin:
//!
//!   * Has no native tree-sitter grammar — `grammar()` returns `None`.
//!   * Emits ONE file-level `Class` symbol named after the dotted
//!     template path (Laravel-style: `users.show` for
//!     `resources/views/users/show.blade.php`), plus one symbol per
//!     directive that names a section / push / stack / component / slot,
//!     and one `Imports` ref per `@extends` / `@include` / `@each` /
//!     `@includeIf` / etc.
//!   * Splits the file into PHP (`{{ … }}`, `{!! … !!}`, `@php … @endphp`)
//!     and JavaScript / TypeScript / CSS / SCSS (`<script>`, `<style>`)
//!     embedded regions for sub-extraction.

pub mod directives;
pub mod embedded;
pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct BladePlugin;

impl LanguagePlugin for BladePlugin {
    fn id(&self) -> &str { "blade" }

    fn language_ids(&self) -> &[&str] { &["blade"] }

    fn extensions(&self) -> &[&str] { &[".blade.php"] }

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
