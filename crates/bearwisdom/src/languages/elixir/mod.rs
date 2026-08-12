//! elixir language plugin.

mod calls;
pub(crate) mod connectors;
mod directives;
pub mod extract;
mod helpers;
pub(crate) mod keywords;
pub(crate) mod phoenix_routes;
mod type_refs;
pub(crate) mod using_injection;
pub(crate) mod predicates;
pub(crate) mod profile;
pub use profile::ELIXIR_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod predicates_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct ElixirPlugin;

impl LanguagePlugin for ElixirPlugin {
    fn id(&self) -> &str {
        "elixir"
    }

    fn language_ids(&self) -> &[&str] {
        &["elixir"]
    }

    fn extensions(&self) -> &[&str] {
        &[".ex", ".exs"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_elixir::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        // Elixir's tree-sitter grammar uses `call` for EVERY expression — module
        // definitions, function definitions, control flow (`if`, `case`, `cond`,
        // `with`, `receive`), and ordinary function invocations alike.  Only ~8%
        // of `call` nodes in real projects are definition-producing, so including
        // "call" in the coverage rules sets a denominator of ~106k against a
        // numerator of ~8k and reports 8% coverage — misleading noise.
        //
        // There is no more specific node kind in the grammar that isolates
        // definitions from invocations.  Symbol coverage is therefore not
        // measurable by tree-sitter node kind for Elixir; returning an empty
        // slice causes the coverage infrastructure to report N/A (percent = -1.0),
        // which the aggregate checker treats as a pass.  Ref coverage (dot + alias)
        // still provides a meaningful correctness signal.
        &[]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["dot", "alias"]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    // TODO(routes-dispatch): wire `connectors::discover_phoenix_routes` into the
    // indexer route-population stage. The function now writes the `routes` table
    // directly (returning the insert count) and the routes-table → FlowEmission
    // bridge in resolve/mod.rs emits the Consumer flows. The
    // `resolve_connection_points` override was removed because the ConnectionPoint
    // Stop emission was redundant with that bridge.

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::ELIXIR_PROFILE)
    }


    fn populate_project_state(
        &self,
        state: &mut crate::indexer::plugin_state::PluginStateBag,
        parsed: &[crate::types::ParsedFile],
        _project_root: &std::path::Path,
        _project_ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        state.set(using_injection::build_using_injection_map(parsed));
    }

    fn populate_project_state_post_externals(
        &self,
        state: &mut crate::indexer::plugin_state::PluginStateBag,
        parsed: &[crate::types::ParsedFile],
        _project_root: &std::path::Path,
        _project_ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        // `use ExUnit.Case`/`use ExUnit.CaseTemplate`'s `__using__`/`using do`
        // quote blocks live in ExUnit's own external source, invisible to the
        // pre-externals pass. Rebuilding here against the now-externals-merged
        // `parsed` slice lets the injection map see them.
        state.set(using_injection::build_using_injection_map(parsed));
    }
}
