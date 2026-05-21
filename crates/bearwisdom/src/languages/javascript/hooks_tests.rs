// =============================================================================
// languages/javascript/hooks_tests.rs — sanity checks for the JS hook skeleton.
// =============================================================================

use super::*;
use crate::type_checker::profile::hooks::LanguageEngineHooks;

#[test]
fn javascript_hooks_is_send_sync() {
    fn require_send_sync<T: Send + Sync + ?Sized>() {}
    require_send_sync::<JavascriptHooks>();
    require_send_sync::<dyn LanguageEngineHooks>();
}

#[test]
fn javascript_plugin_exposes_hooks() {
    use crate::languages::LanguagePlugin;
    let plugin = crate::languages::javascript::JavascriptPlugin;
    assert!(
        plugin.language_hooks().is_some(),
        "JS plugin must register engine hooks so embedded-JS refs in Vue 2 SFCs \
         and standalone .js files reach the TS lib-globals fallback"
    );
}
