// =============================================================================
// type_checker/mod.rs — type-system foundation for the resolution engine
//
// What remains after the legacy resolver was deleted: the shared type model.
//   - core::types  — Type / TypeId / TypeArena, the interned type universe the
//                     extractors populate and the new engine resolves against.
//   - profile      — per-language `LanguageProfile` data (kind tables, syntax
//                     axes) the engine's rule ladder and chain walker consult.
//
// Resolution itself lives in `indexer/resolve/engine/`. The old per-language
// TypeChecker/LanguageResolver hooks and the DefaultResolver chain/inheritance/
// alias/subtype machinery were removed with the legacy engine.
// =============================================================================

pub mod core;
pub mod profile;
