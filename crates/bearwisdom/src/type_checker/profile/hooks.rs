// =============================================================================
// type_checker/profile/hooks.rs — LanguageEngineHooks trait
//
// Escape hatch for per-language behaviour that can't be expressed as
// LanguageProfile data: reshaping refs whose syntactic form doesn't match
// the canonical contract (ObjC bracket calls), enriching external types
// with information not present in their source, special dispatch (Haskell
// typeclass, R S4), custom flow emission.
//
// All methods have no-op defaults. The default impl `NoOpHooks` is what the
// engine binds when a language plugin does not ship its own hooks.
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, RefContext as ResolveRefContext, Resolution, SymbolLookup,
};
use crate::types::ExtractedRef;

use super::super::core::types::{TypeArena, TypeId};

/// Per-resolution context the dispatch hook can inspect to pick among
/// candidates.
#[derive(Debug)]
pub struct DispatchContext<'a> {
    /// The unresolved Calls ref the engine is dispatching.
    pub call_ref: &'a ExtractedRef,
    /// Argument TypeIds inferred so far (positional). Empty when the engine
    /// hasn't inferred any.
    pub arg_types: &'a [TypeId],
    /// Expected return type from the call site (None when free-standing).
    pub expected_return: Option<TypeId>,
}

/// Per-ref context the flow-emission hook can inspect when deciding whether
/// to emit a cross-tier `FlowEmission`.
#[derive(Debug)]
pub struct RefContext<'a> {
    /// Canonical file path the ref lives in.
    pub file_path: &'a str,
    /// Language id (`ParsedFile.language`).
    pub language: &'a str,
    /// Owning symbol's qualified_name.
    pub owner_qname: Option<&'a str>,
}

/// Stand-in target for `resolve_dispatch_special`. The hook returns the DB
/// symbol id of the implementation to bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchTarget {
    pub symbol_id: i64,
}

/// Outcome surfaced by `detect_flow_emission_special`. The engine treats it
/// the same way it treats flow emissions surfaced by chain rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomFlowEmission {
    pub kind: &'static str,
    pub target_qname: Option<String>,
    pub protocol: Option<&'static str>,
    pub confidence: u8,
}

/// Behaviour a language plugin may override beyond what `LanguageProfile`
/// expresses as data. All methods have no-op defaults; outliers override
/// only the methods they need.
pub trait LanguageEngineHooks: Send + Sync {
    /// Reshape an `ExtractedRef` whose syntactic form differs from the
    /// canonical contract. ObjC's `[obj msg:arg]` flattens to chain form
    /// via this hook.
    fn preprocess_ref(&self, _ref_: &mut ExtractedRef) {}

    /// Enrich an externally-sourced type with detail not present in its
    /// declaration (Rust associated-type bindings, TypeScript declaration
    /// merging). Default: no-op.
    fn enrich_external_type(&self, _ty: TypeId, _arena: &TypeArena) {}

    /// Pick a dispatch target for non-receiver dispatch axes. Returns the
    /// DB symbol id of the chosen target, or `None` when the hook can't
    /// decide (engine then falls back to receiver dispatch).
    fn resolve_dispatch_special(
        &self,
        _candidate_symbol_ids: &[i64],
        _ctx: &DispatchContext<'_>,
    ) -> Option<DispatchTarget> {
        None
    }

    /// Detect a cross-tier flow emission the chain-walker rules can't
    /// express. Default: no emission.
    fn detect_flow_emission_special(
        &self,
        _ref_: &ExtractedRef,
        _ctx: &RefContext<'_>,
    ) -> Option<CustomFlowEmission> {
        None
    }

    /// Resolve a chain-less ref BEFORE the engine's generic bare-name
    /// strategies run. Used for language-specific behaviors that take
    /// priority over the generic scope-chain / same-file / qname path:
    /// TypeScript workspace-package imports and tsconfig path aliases,
    /// Python relative-import resolution, Go package-qualifier rewrites.
    /// Returns `Some(Resolution)` to short-circuit; `None` to defer to
    /// the engine's generic path.
    fn resolve_bare_pre(
        &self,
        _ref_ctx: &ResolveRefContext<'_>,
        _file_ctx: &FileContext,
        _lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        None
    }

    /// Resolve a chain-less ref AFTER the engine's generic bare-name
    /// strategies declined. Used for language-specific fallback paths
    /// that don't compete with the generic resolution (TS npm globals
    /// and core-lib ambient-global fallback, .NET extension-method
    /// search across using directives). Returns `Some(Resolution)` to
    /// recover an otherwise-unresolved ref; `None` lets the resolver
    /// loop fall through to its legacy/heuristic tier.
    fn resolve_bare_post(
        &self,
        _ref_ctx: &ResolveRefContext<'_>,
        _file_ctx: &FileContext,
        _lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        None
    }

    /// Classify an unresolved ref as belonging to an external namespace
    /// (third-party package, language runtime, framework). Returns the
    /// external namespace string the engine writes to `externals` (e.g.
    /// `"ext:react"`, `"@types/node"`, `"Microsoft.EntityFrameworkCore"`).
    ///
    /// Engine consults this hook in the Tier 1.5 block of the resolver
    /// loop (after engine + legacy resolution failed, before the
    /// chain-walker + name-classifier + import-table generic checks).
    /// Default returns `None` — the engine's generic external paths
    /// (chain inference, primitive/builtin classification, import-table
    /// inference) still run. Per-language hooks override to express
    /// ecosystem-specific rules: TS workspace-package membership,
    /// Python site-packages, Java pom.xml/Gradle, C# NuGet/.csproj,
    /// Go go.mod, etc.
    fn classify_external(
        &self,
        _ref_ctx: &ResolveRefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        None
    }

    /// Detect cross-tier flow-emission patterns (HTTP client calls, IPC,
    /// WebSocket emits, etc.) for a `Calls`-kind ref. Independent of whether
    /// the symbol resolved — emission rules key on import context + chain
    /// shape, not on a resolved target. Default: no emission.
    fn detect_flow_emissions(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &ResolveRefContext<'_>,
        _lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        Vec::new()
    }

    /// Construct the per-file resolution context. Languages override this to
    /// thread import statements, file namespace, package id, embedded
    /// regions, etc. — the things resolution depends on but that aren't
    /// derivable from the ref alone. Returns `None` to let the engine fall
    /// through to the legacy `LanguageResolver::build_file_context` path.
    fn build_file_context(
        &self,
        _file: &crate::types::ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        None
    }

    /// Resolve a ref via the language-specific rules. Returns `Some(Resolution)`
    /// when the language has a deterministic answer, `None` to let the engine
    /// fall through to other strategies. Default: no resolution.
    fn resolve_ref(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &ResolveRefContext<'_>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        None
    }

    /// Override the chain walker's root-segment type resolution. Returns a
    /// `&dyn RootResolver` the engine threads through `walk_with_root`;
    /// `None` keeps the engine's `DefaultRootResolver`.
    ///
    /// This is the hook for frameworks where `this` (or another self
    /// keyword) has an *implicit* type set by the framework — Vue 2/3
    /// component instances, Vuex action contexts, MDX page contexts —
    /// rather than by user-source declarations the extractor already
    /// captured.
    ///
    /// The resolver impl should discover the implicit type structurally
    /// from the symbol index (looking for the canonical member set the
    /// framework declares in its `.d.ts`) so version changes that rename
    /// the type do not require code changes here. Hardcoding qnames or
    /// branching on package versions is a smell — let the framework's
    /// own type declarations drive the answer.
    fn root_resolver(
        &self,
    ) -> Option<&'static dyn crate::type_checker::core::chain::RootResolver> {
        None
    }
}

/// Concrete no-op implementation. Bound by the engine when a language
/// plugin does not ship its own hooks.
pub struct NoOpHooks;

impl LanguageEngineHooks for NoOpHooks {}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
