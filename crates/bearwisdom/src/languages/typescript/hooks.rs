// =============================================================================
// languages/typescript/hooks.rs — TypeScriptHooks impl of LanguageEngineHooks
//
// Phase 5 deliverable: the per-language hook surface for TypeScript /
// TSX / JSX / JS. Skeleton today — every method returns the trait default
// so engine behaviour is unchanged. Future wave-A/B migrations fill in:
//   - synthesize_members for `@Component` (Angular), `@Entity` (TypeORM),
//     class-mixin returns.
//   - enrich_external_type for declaration merging (interface X augments
//     existing X across files).
//   - detect_flow_emission_special for special HTTP-client patterns the
//     resolver-side detector can't express.
//
// The trait's no-op defaults mean this skeleton is correct *now*: engine
// resolve runs identically with or without it. The point of landing it in
// Phase 5 is to give Phase 6 wave-A languages (Python, Java, C#, Go) a
// reference shape to follow.
// =============================================================================

use crate::type_checker::profile::hooks::LanguageEngineHooks;

/// TypeScript engine hooks. Today inherits every default; ready to be
/// extended by per-feature migrations.
pub struct TypeScriptHooks;

impl LanguageEngineHooks for TypeScriptHooks {}

/// Static instance the language plugin returns. `'static` so the engine
/// can store `&'static dyn LanguageEngineHooks` in its hook registry
/// without lifetime gymnastics.
pub static TYPESCRIPT_HOOKS: TypeScriptHooks = TypeScriptHooks;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
