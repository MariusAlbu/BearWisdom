// =============================================================================
// languages/typescript/hooks.rs — TypeScriptHooks impl of LanguageEngineHooks.
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
