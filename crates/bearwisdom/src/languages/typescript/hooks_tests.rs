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

#[test]
fn synthesize_members_returns_empty_by_default() {
    let hooks = TypeScriptHooks;
    let arena = crate::type_checker::core::TypeArena::new();
    let result = hooks.synthesize_members("Foo", &[], &arena);
    assert!(result.is_empty());
}

#[test]
fn static_instance_matches_struct_behaviour() {
    let arena = crate::type_checker::core::TypeArena::new();
    assert!(TYPESCRIPT_HOOKS
        .synthesize_members("Foo", &[], &arena)
        .is_empty());
}
