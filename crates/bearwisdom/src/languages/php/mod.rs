//! php language plugin.
mod callback_lexical;

mod calls;
pub(crate) mod decorators;
pub mod embedded;
pub mod extract;
pub(crate) mod flow;
mod helpers;
pub(crate) mod module_paths;
mod imports;
pub(crate) mod keywords;
mod property_decl;
mod symbols;
mod type_ref_emit;

pub mod connectors;
mod predicates;
pub(crate) mod profile;
pub use profile::PHP_PROFILE;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod calls_tests;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod predicates_tests;

#[cfg(test)]
#[path = "property_decl_tests.rs"]
mod property_decl_tests;

#[cfg(test)]
#[path = "type_ref_emit_tests.rs"]
mod type_ref_emit_tests;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct PhpPlugin;

impl LanguagePlugin for PhpPlugin {
    fn id(&self) -> &str {
        "php"
    }

    fn language_ids(&self) -> &[&str] {
        &["php"]
    }

    fn extensions(&self) -> &[&str] {
        &[".php"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_php::LANGUAGE_PHP.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::PHP_SCOPE_KINDS
    }

    fn callback_lexical_adapter(
        &self,
    ) -> Option<&'static crate::indexer::callback_lexical::CallbackLexicalAdapter> {
        Some(&callback_lexical::ADAPTER)
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn type_text_policy(&self) -> crate::languages::TypeTextPolicy {
        crate::languages::TypeTextPolicy {
            thin_arrow_function: true,
            nullable_prefix: true,
            nullable_suffix: true,
            array_suffix: true,
            union_intersection: true,
            angle_application: true,
            ..crate::languages::TypeTextPolicy::OPAQUE
        }
    }

    /// Namespace-qualified spellings inside a PHP type expression
    /// (`\App\Column`, `?Foo\Bar`, `A\B|null`) intern as the canonical index
    /// qname of the declaration they name.
    fn intern_type_text(
        &self,
        arena: &crate::type_checker::core::types::TypeArena,
        text: &str,
    ) -> crate::type_checker::core::types::TypeId {
        let canonical = helpers::canonicalize_type_text(text);
        crate::languages::type_text::intern_type_text(arena, &canonical, self.type_text_policy())
    }

    fn signature_return_type(&self, signature: &str) -> Option<String> {
        helpers::signature_return_type(signature)
    }

    fn signature_parameter_types(&self, signature: &str) -> Option<Vec<String>> {
        helpers::signature_parameter_types(signature)
    }

    /// E2: surface `<script>` and `<style>` blocks that live in the HTML
    /// regions between `<?php … ?>` blocks for sub-extraction by the JS,
    /// TS, CSS, and SCSS plugins. Pure-PHP files (no HTML mode) emit
    /// nothing.
    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "interface_declaration",
            "trait_declaration",
            "enum_declaration",
            "enum_case",
            "function_definition",
            "method_declaration",
            "property_declaration",
            "const_declaration",
            "namespace_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "function_call_expression",
            "member_call_expression",
            "nullsafe_member_call_expression",
            "scoped_call_expression",
            "object_creation_expression",
            "namespace_use_declaration",
            "use_declaration",
            "base_clause",
            "class_interface_clause",
            "attribute",
            "named_type",
            "union_type",
            "intersection_type",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::PHP_PROFILE)
    }

    fn discover_routes(
        &self,
        conn: &rusqlite::Connection,
        project_root: &std::path::Path,
        project_ctx: &crate::indexer::project_context::ProjectContext,
    ) -> u32 {
        connectors::discover_laravel_routes(conn, project_root, project_ctx)
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::PHP_FLOW_CONFIG)
    }

    fn normalize_flow_guard_type(&self, raw: &str) -> Option<String> {
        crate::languages::common::normalize_identifier_capture(raw)
    }

    fn flow_cfg_node_kinds(&self) -> Option<&'static crate::indexer::flow_cfg::CfgNodeKinds> {
        Some(&flow::PHP_CFG_KINDS)
    }

    fn flow_return_query(&self) -> Option<&'static str> {
        Some(flow::PHP_RETURN_QUERY)
    }
}
