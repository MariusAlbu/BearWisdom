//! Vue language plugin.
//!
//! Vue Single File Components (.vue) are parsed using the HTML grammar as a
//! structural fallback. There is no tree-sitter-vue in the workspace yet.
//!
//! What we extract at this grammar level:
//! - The file itself → a Class symbol (component name inferred from filename)
//! - PascalCase tags in the template → Calls edges (component usages)
//! - Kebab-case custom element tags (contains at least one hyphen) → Calls edges
//!   (normalized to PascalCase)
//! - Event handler directives (@event / v-on) → Calls edges to the handler method
//!
//! The <script> block's JS/TS symbols are handled by the JS/TS extractor when
//! the indexer processes the embedded text as a separate extraction target.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct VuePlugin;

impl LanguagePlugin for VuePlugin {
    fn id(&self) -> &str {
        "vue"
    }

    fn language_ids(&self) -> &[&str] {
        &["vue"]
    }

    fn extensions(&self) -> &[&str] {
        &[".vue"]
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
        // Vue SFC: symbols come from script block; at the grammar level only
        // the component-level element is meaningful.
        &["script_element", "template_element"]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        // Component invocations and event handler directives.
        &["element", "self_closing_tag", "directive_attribute"]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }
}
