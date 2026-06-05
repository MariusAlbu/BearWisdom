//! elixir language plugin.

mod helpers;
pub(crate) mod connectors;
pub(crate) mod phoenix_routes;
pub(crate) mod keywords;
pub mod extract;
mod calls;
mod directives;
mod type_refs;

pub(crate) mod hooks;
pub(crate) mod predicates;
pub(crate) mod profile;

pub use hooks::ELIXIR_HOOKS;
pub use profile::ELIXIR_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub struct ElixirPlugin;

impl LanguagePlugin for ElixirPlugin {
    fn id(&self) -> &str { "elixir" }

    fn language_ids(&self) -> &[&str] { &["elixir"] }

    fn extensions(&self) -> &[&str] { &[".ex", ".exs"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_elixir::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

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
        &[
            "dot",
            "alias",
        ]
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

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&hooks::ELIXIR_HOOKS)
    }
}