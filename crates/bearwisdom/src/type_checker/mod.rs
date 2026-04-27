// =============================================================================
// type_checker/mod.rs — First-class type checker for BearWisdom
//
// Promotes type checking to a peer module of `ecosystem/` and `indexer/`.
// One TypeChecker impl per language; shared algorithms (chain walking,
// inheritance walking, generic substitution, alias expansion) move here as
// default trait methods or supporting modules under this directory.
//
// Why now: type-related logic was scattered across
//   - indexer/resolve/chain_walker.rs (unified chain walker)
//   - indexer/resolve/inheritance.rs (resolve_via_inheritance)
//   - indexer/resolve/type_env.rs (TypeEnvironment)
//   - languages/typescript/chain.rs (TS-specific chain walker)
//   - languages/csharp/chain.rs, languages/go/chain.rs (more chain variants)
//   - languages/*/predicates.rs (kind_compatible per language)
//   - indexer/resolve/engine.rs (infer_external_from_chain, external_type_qname)
//
// Single Responsibility says one module owns type checking. Adding new
// type-level operations (alias expansion, keyof/typeof, mapped types,
// conditional types) into the scattered structure would worsen the smell;
// consolidating before adding them is the cheaper sequence.
//
// PR 1 (this commit) establishes the contract + registry hook only.
// Behavior migrates in subsequent PRs:
//   PR 2 — port unified chain walker as default `resolve_chain` impl.
//   PR 3 — collapse languages/typescript/chain.rs into TypeScriptChecker.
//   PR 4 — port inheritance.rs as `walk_inheritance` default.
//   PR 5 — port per-language `kind_compatible` predicates.
//   PR 6+ — add type-level computation: alias expansion, keyof, typeof,
//           mapped types, conditional types.
//
// See decision-2026-04-27-e75 in the knowledge memory for full rationale.
// =============================================================================

pub mod type_env;

pub use type_env::TypeEnvironment;

use std::sync::Arc;

/// First-class type checker for a single language.
///
/// One impl per language (TypeScriptChecker, PythonChecker, etc.). Each
/// language plugin returns its checker via `LanguagePlugin::type_checker()`.
/// The engine keeps a registry keyed by `language_id`, mirroring how
/// `LanguageResolver` is registered today.
///
/// The trait surface stays empty in PR 1 except for the identity hook —
/// behavior migrates in subsequent PRs as documented in this module's header.
/// This way PR 1 is purely structural: types compile, tests pass, no behavior
/// changes, and the seam is in place for incremental porting.
pub trait TypeChecker: Send + Sync {
    /// The language identifier this checker handles. Engine uses this as the
    /// registry key, matching the language id strings returned by
    /// `LanguageResolver::language_ids()` (which a checker typically claims a
    /// subset of — e.g. TypeScriptChecker handles "typescript" but not "tsx",
    /// at least for the type-level operations TS shares with JS).
    fn language_id(&self) -> &str;
}

/// Aggregate the type checkers from every registered language plugin.
/// Plugins without a checker return `None` (most non-typed languages today).
/// Mirrors `crate::languages::default_resolvers()` in shape.
pub fn default_type_checkers() -> Vec<Arc<dyn TypeChecker>> {
    crate::languages::default_registry()
        .all()
        .iter()
        .filter_map(|plugin| plugin.type_checker())
        .collect()
}
