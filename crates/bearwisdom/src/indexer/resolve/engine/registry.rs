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
    ///
    /// Default returns a minimal placeholder; languages migrating onto the
    /// LanguageEngineHooks path satisfy this trait via the default and the
    /// real construction lives in `hooks::build_file_context`.
    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        FileContext {
            file_path: file.path.clone(),
            language: file.language.clone(),
            imports: Vec::new(),
            file_namespace: None,
        }
    }

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

/// Vestigial dispatcher kept for ABI compatibility; resolution flows entirely
/// through `LanguageEngineHooks`. The struct holds no state.
pub struct ResolutionEngine;

impl ResolutionEngine {
    pub fn new() -> Self {
        Self
    }
}
