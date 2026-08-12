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

use crate::indexer::plugin_state::PluginStateBag;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EdgeKind, ExtractionResult, ParsedFile};

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

    /// One-hop `use M` redirect: for each wildcard-eligible `Imports` ref
    /// whose module M defines `__using__`/`using do` (per
    /// `ElixirProjectState`), turn M's own `import`/`alias` injection
    /// directives into synthetic imports for the `use`ing file.
    ///
    /// `alias`/`require` directives never invoke `__using__`, so they're
    /// excluded via `is_import_binding`. A plain `import M` is structurally
    /// identical to `use M` at the ref level — Elixir's extractor emits the
    /// same shape for both — but `injections_for` only returns entries for
    /// modules that actually define the macro, and real code only ever
    /// `use`s such a module rather than `import`ing it, so that lookup
    /// doubles as the `use`-site filter without a separate directive-kind
    /// field on `ExtractedRef`.
    fn extra_wildcard_imports(&self, state: &PluginStateBag, file: &ParsedFile) -> Vec<ImportEntry> {
        let Some(project_state) = state.get::<using_injection::ElixirProjectState>() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for r in &file.refs {
            if r.kind != EdgeKind::Imports || r.is_import_binding {
                continue;
            }
            let Some(used_module) = r.module.as_deref() else {
                continue;
            };
            let Some(injections) = project_state.injections_for(used_module) else {
                continue;
            };
            for inj in injections {
                match inj {
                    using_injection::ElixirInjection::Import { module } => {
                        let name = module.rsplit('.').next().unwrap_or(module).to_string();
                        out.push(ImportEntry {
                            imported_name: name,
                            module_path: Some(module.clone()),
                            alias: None,
                            is_wildcard: true,
                        });
                    }
                    using_injection::ElixirInjection::Alias { local, module } => {
                        // Elixir's own directive extraction bakes an `as:`
                        // rename directly into `imported_name` (its refs
                        // never carry a `chain`, so `build_file_context`'s
                        // alias-detection never fires) — mirror that shape
                        // here so `AliasModuleQnameRule` matches on the
                        // local bound name the same way it does for a
                        // hand-written `alias M.Foo, as: Bar`.
                        out.push(ImportEntry {
                            imported_name: local.clone(),
                            module_path: Some(module.clone()),
                            alias: None,
                            is_wildcard: false,
                        });
                    }
                    // A nested `use N` inside M's own quote block is a
                    // second hop — left for a future transitive expansion.
                    using_injection::ElixirInjection::Use { .. } => {}
                }
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod mod_tests;
