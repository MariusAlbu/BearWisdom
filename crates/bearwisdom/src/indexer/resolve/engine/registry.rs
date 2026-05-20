// =============================================================================
// indexer/resolve/engine/registry.rs — per-language resolver trait + dispatch
//
// The `LanguageResolver` trait is the per-language contract: every language
// plugin that wants deterministic (tier-1) resolution implements it and
// returns its impl from `LanguagePlugin::resolver()`. The `ResolutionEngine`
// struct collects those impls keyed by language id and hands them to the
// resolve loop via `resolver_for()`.
// =============================================================================

use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::indexer::project_context::ProjectContext;
use crate::types::ParsedFile;

use super::{FileContext, RefContext, Resolution, SymbolInfo, SymbolLookup};

// ---------------------------------------------------------------------------
// LanguageResolver trait
// ---------------------------------------------------------------------------

/// Per-language resolution rules. Each language implements this trait
/// in a separate file under `resolve/rules/`.
pub trait LanguageResolver: Send + Sync {
    /// The language identifier(s) this resolver handles.
    /// Must match the language strings from file detection.
    fn language_ids(&self) -> &[&str];

    /// Build the file context for a parsed file.
    /// `project_ctx` provides global usings and external prefix data.
    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext;

    /// Attempt to resolve a reference using language-specific scope rules.
    ///
    /// Returns `Some(Resolution)` if deterministically resolved.
    /// Returns `None` to fall back to the heuristic resolver.
    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution>;

    /// Check whether a target symbol is visible from the reference site.
    /// Default: always visible (no filtering).
    fn is_visible(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _target: &SymbolInfo,
    ) -> bool {
        true
    }

}

// ---------------------------------------------------------------------------
// ResolutionEngine
// ---------------------------------------------------------------------------

/// The engine that dispatches resolution to language-specific resolvers.
pub struct ResolutionEngine {
    resolvers: FxHashMap<String, Arc<dyn LanguageResolver>>,
}

impl ResolutionEngine {
    /// Create a new engine with the default set of language resolvers.
    pub fn new() -> Self {
        let mut engine = Self {
            resolvers: FxHashMap::default(),
        };
        for resolver in crate::languages::default_resolvers() {
            for &lang_id in resolver.language_ids() {
                engine
                    .resolvers
                    .insert(lang_id.to_string(), Arc::clone(&resolver));
            }
        }
        engine
    }

    /// Get the resolver for a language, if one is registered.
    pub fn resolver_for(&self, language: &str) -> Option<&dyn LanguageResolver> {
        self.resolvers.get(language).map(|r| r.as_ref())
    }
}
