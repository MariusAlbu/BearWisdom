//! python language plugin.
mod callback_lexical;

mod assignments;
mod calls;
pub(crate) mod connectors;
pub(crate) mod decorators;
mod embedded;
mod external_virtual_path;
pub mod extract;
pub(crate) mod flow;
mod helpers;
mod imports;
pub(crate) mod keywords;
mod statements;
mod symbols;
mod types;

mod externals;
mod predicates;
pub mod profile;
pub use profile::PYTHON_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod calls_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod predicates_tests;

use crate::ecosystem::manifest::ManifestKind;
use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct PythonPlugin;

impl LanguagePlugin for PythonPlugin {
    fn id(&self) -> &str {
        "python"
    }

    fn language_ids(&self) -> &[&str] {
        &["python"]
    }

    fn extensions(&self) -> &[&str] {
        &[".py", ".pyi"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_python::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn external_virtual_path(
        &self,
        _language: &str,
        normalized_absolute_path: &str,
    ) -> Option<String> {
        external_virtual_path::for_pulled(normalized_absolute_path)
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
            "class_definition",
            "function_definition",
            // `decorated_definition` wraps class_definition/function_definition;
            // those inner node kinds already cover decorated defs when the
            // start_line is not patched to the decorator line.
            "assignment",
            "type_alias_statement",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call",
            "import_statement",
            "import_from_statement",
            "future_import_statement",
            "typed_parameter",
            "typed_default_parameter",
            "type",
            "generic_type",
            "union_type",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn signature_type_application(&self, text: &str) -> (String, Vec<String>) {
        crate::languages::bracket_type_application(text)
    }

    fn signature_type_head<'a>(&self, text: &'a str) -> &'a str {
        crate::languages::bracket_type_head(text)
    }

    fn signature_parameter_types(&self, signature: &str) -> Option<Vec<String>> {
        crate::languages::colon_parameter_types(signature)
    }

    fn source_module_path_policy(
        &self,
        _specifier: &str,
    ) -> crate::type_checker::profile::language_profile::SourceModulePathPolicy {
        predicates::SOURCE_MODULE_PATH_POLICY
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&PYTHON_PROFILE)
    }

    // TODO(routes-dispatch): wire `connectors::discover_django_routes` and
    // `connectors::discover_fastapi_routes` into the indexer route-population
    // stage. Both functions now write the `routes` table directly (returning
    // the insert count) and the routes-table → FlowEmission bridge in
    // resolve/mod.rs emits the Consumer flows. The `resolve_connection_points`
    // override was removed because the ConnectionPoint Stop emission was
    // redundant with that bridge.

    fn post_index(
        &self,
        db: &crate::db::Database,
        project_root: &std::path::Path,
        ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        if ctx.has_dependency(ManifestKind::PyProject, "django") {
            connectors::run_django_concepts(db, project_root);
        }
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::PY_FLOW_CONFIG)
    }

    fn normalize_flow_guard_type(&self, raw: &str) -> Option<String> {
        crate::languages::common::normalize_identifier_capture(raw)
    }

    fn flow_cfg_node_kinds(&self) -> Option<&'static crate::indexer::flow_cfg::CfgNodeKinds> {
        Some(&flow::PYTHON_CFG_KINDS)
    }

    fn flow_return_query(&self) -> Option<&'static str> {
        Some(flow::PY_RETURN_QUERY)
    }

    fn plugin_flow_emissions(
        &self,
        source: &str,
        _file_path: &str,
    ) -> Vec<(u32, crate::indexer::resolve::flow_emit::FlowEmission)> {
        connectors::extract_python_graphql(source)
    }
}
