// =============================================================================
// languages/go/hooks.rs — GoHooks impl of LanguageEngineHooks.
// =============================================================================

use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct GoHooks;

impl LanguageEngineHooks for GoHooks {}

pub static GO_HOOKS: GoHooks = GoHooks;
