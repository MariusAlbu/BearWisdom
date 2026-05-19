// =============================================================================
// languages/typescript/hooks_tests.rs — sanity checks for the hook skeleton.
// =============================================================================

use super::*;
use crate::type_checker::profile::hooks::LanguageEngineHooks;

#[test]
fn typescript_hooks_is_send_sync() {
    fn require_send_sync<T: Send + Sync + ?Sized>() {}
    require_send_sync::<TypeScriptHooks>();
    require_send_sync::<dyn LanguageEngineHooks>();
}
