//! Svelte language plugin.
//!
//! Svelte Single File Components (.svelte) are parsed using the HTML grammar as a
//! structural fallback — the same approach used for Vue. There is no
//! tree-sitter-svelte in the workspace.
//!
//! What we extract at this grammar level:
//! - The file itself → a Class symbol (component name inferred from filename)
//! - PascalCase tags in the template → Calls edges (component usages)
//! - Kebab-case custom element tags (contains at least one hyphen) → Calls edges
//!   (normalised to PascalCase)
//! - `on:event` handler directives → Calls edges to the handler function
//! - `{#if}` / `{#each}` block markers → recorded as Calls to their identifiers
//!   when a function reference is present (e.g., `{#each items as item}` → Calls "items")
//!
//! The `<script>` block's JS/TS symbols are handled by the JS/TS extractor when
//! the indexer processes the embedded text as a separate extraction target.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct SveltePlugin;

impl LanguagePlugin for SveltePlugin {
    fn id(&self) -> &str {
        "svelte"
    }

    fn language_ids(&self) -> &[&str] {
        &["svelte"]
    }

    fn extensions(&self) -> &[&str] {
        &[".svelte"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        // HTML grammar handles the SFC outer shell (template/script/style tags).
        Some(tree_sitter_html::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source, file_path)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &["script_element", "element"]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["element", "self_closing_element", "attribute"]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }
}
