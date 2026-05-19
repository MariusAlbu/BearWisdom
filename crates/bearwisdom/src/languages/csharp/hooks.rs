// =============================================================================
// languages/csharp/hooks.rs — CSharpHooks impl of LanguageEngineHooks.
// =============================================================================

use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct CSharpHooks;

impl LanguageEngineHooks for CSharpHooks {}

pub static CSHARP_HOOKS: CSharpHooks = CSharpHooks;
