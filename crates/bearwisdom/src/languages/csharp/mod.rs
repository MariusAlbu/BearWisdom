//! csharp language plugin.

mod calls;
mod calls_narrowing;
mod calls_routes;
mod calls_symbols;
pub mod connectors;
pub(crate) mod decorators;
mod embedded;
pub mod extract;
pub(crate) mod flow;
mod helpers;
pub(crate) mod keywords;
mod symbols;
mod types;

pub mod hooks;
mod predicates;
pub mod profile;
mod source_gen;
pub use hooks::CSHARP_HOOKS;
pub use profile::CSHARP_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "source_gen_tests.rs"]
mod source_gen_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod calls_tests;

use crate::languages::{LanguagePlugin, Synthesized};
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractedRef, ExtractedSymbol, ExtractionResult};

pub struct CSharpPlugin;

impl LanguagePlugin for CSharpPlugin {
    fn id(&self) -> &str {
        "csharp"
    }

    fn language_ids(&self) -> &[&str] {
        &["csharp"]
    }

    fn extensions(&self) -> &[&str] {
        &[".cs"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_c_sharp::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::CSHARP_SCOPE_KINDS
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        let mut result = extract::extract(source);
        crate::languages::common::append_handlebars_register_helper_globals(source, &mut result);
        result
    }

    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(source)
    }

    fn synthesize_symbols(
        &self,
        source: &str,
        symbols: &[ExtractedSymbol],
        refs: &[ExtractedRef],
    ) -> Synthesized {
        source_gen::synthesize_symbols(source, symbols, refs)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "struct_declaration",
            "record_declaration",
            "interface_declaration",
            "enum_declaration",
            "enum_member_declaration",
            "delegate_declaration",
            "event_declaration",
            "event_field_declaration",
            "method_declaration",
            "constructor_declaration",
            "destructor_declaration",
            "property_declaration",
            "indexer_declaration",
            "operator_declaration",
            "conversion_operator_declaration",
            "field_declaration",
            "local_function_statement",
            "namespace_declaration",
            "file_scoped_namespace_declaration",
            "accessor_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "invocation_expression",
            "object_creation_expression",
            "implicit_object_creation_expression",
            "using_directive",
            "base_list",
            "type_argument_list",
            "cast_expression",
            "is_expression",
            "as_expression",
            "typeof_expression",
            "attribute",
            "generic_name",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&CSHARP_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks> {
        Some(&CSHARP_HOOKS)
    }

    fn post_index(
        &self,
        db: &crate::db::Database,
        _project_root: &std::path::Path,
        _ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        if let Err(e) = connectors::run_ef_core(db) {
            tracing::warn!("EF Core post-index hook: {e}");
        }
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::CSHARP_FLOW_CONFIG)
    }
}
