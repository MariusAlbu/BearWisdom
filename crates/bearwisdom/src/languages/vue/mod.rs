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

pub mod auto_import_dts;
pub mod connectors;
pub mod extract;
pub mod global_registry;
pub(crate) mod hooks;
pub(crate) mod predicates;
pub(crate) mod profile;

pub use hooks::VUE_HOOKS;
pub use profile::VUE_PROFILE;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::indexer::plugin_state::PluginStateBag;
use crate::indexer::project_context::ProjectContext;
use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult, ParsedFile};

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

    /// Split out `<script>` / `<script setup lang="ts">` and `<style>` blocks
    /// for sub-extraction by the JS/TS/CSS/SCSS plugins. Indexer splices the
    /// resulting symbols/refs back into the same `.vue` file.
    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        crate::languages::common::extract_html_script_style_regions(source)
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

    fn keywords(&self) -> &'static [&'static str] {
        crate::languages::typescript::keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::VUE_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks> {
        Some(&hooks::VUE_HOOKS)
    }
    fn populate_project_state(
        &self,
        state: &mut PluginStateBag,
        parsed: &[ParsedFile],
        project_root: &std::path::Path,
        _project_ctx: &ProjectContext,
    ) {
        let parsed_paths: Vec<String> = parsed
            .iter()
            .filter(|pf| !pf.path.starts_with("ext:"))
            .map(|pf| pf.path.clone())
            .collect();
        let registry = global_registry::scan_global_registrations(project_root, &parsed_paths);
        if !registry.is_empty() {
            tracing::info!(
                "Vue global registry: {} components/prefixes, unplugin_auto_import={}",
                registry.components.len(),
                registry.has_unplugin_auto_import,
            );
        }
        state.set(registry);
    }
}
