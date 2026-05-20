//! python language plugin.

mod calls;
pub(crate) mod connectors;
pub(crate) mod decorators;
mod embedded;
mod flow;
mod helpers;
pub(crate) mod keywords;
mod symbols;
mod assignments;
mod statements;
mod types;
pub mod extract;

pub mod hooks;
mod predicates;
pub mod profile;
pub(crate) mod type_checker;
mod externals;
mod flow_detectors;

pub use hooks::PYTHON_HOOKS;
pub use hooks::PythonResolver;
pub use profile::PYTHON_PROFILE;

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
#[path = "resolve_tests.rs"]
mod resolve_tests;

#[cfg(test)]
#[path = "flow_tests.rs"]
mod flow_tests;

use crate::ecosystem::manifest::ManifestKind;
use crate::languages::LanguagePlugin;
use crate::types::{EmbeddedRegion, ExtractionResult};
use crate::parser::scope_tree::ScopeKind;

pub struct PythonPlugin;

impl LanguagePlugin for PythonPlugin {
    fn id(&self) -> &str { "python" }

    fn language_ids(&self) -> &[&str] { &["python"] }

    fn extensions(&self) -> &[&str] { &[".py", ".pyi"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_python::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

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

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&PYTHON_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&PYTHON_HOOKS)
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
}