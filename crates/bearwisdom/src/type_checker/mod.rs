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

pub mod alias;
pub mod chain;
pub mod core;
pub mod engine;
pub mod inheritance;
pub mod profile;
pub mod subtype;
pub mod type_env;

pub use engine::Engine;
pub use type_env::TypeEnvironment;

// `TypeChecker` trait deleted — the trait surface collapsed during the
// LanguageEngineHooks consolidation. Per-language type-system logic lives on
// inherent impls in `languages/<lang>/type_checker.rs` and is reached through
// `LanguageEngineHooks` or the unified chain walker.
