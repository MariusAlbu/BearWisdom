// =============================================================================
// languages/java/hooks.rs — JavaHooks impl of LanguageEngineHooks.
// =============================================================================

use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct JavaHooks;

impl LanguageEngineHooks for JavaHooks {}

pub static JAVA_HOOKS: JavaHooks = JavaHooks;
