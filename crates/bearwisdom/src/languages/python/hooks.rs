// =============================================================================
// languages/python/hooks.rs — PythonHooks impl of LanguageEngineHooks.
//
// Phase 6 wave-A skeleton. Inherits every trait default today; future
// migrations fill in synthesize_members for @dataclass / @attr.s, ABCMeta-
// generated stubs, NamedTuple field synthesis, etc.
// =============================================================================

use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct PythonHooks;

impl LanguageEngineHooks for PythonHooks {}

pub static PYTHON_HOOKS: PythonHooks = PythonHooks;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_checker::core::TypeArena;

    #[test]
    fn synthesize_members_returns_empty_by_default() {
        let arena = TypeArena::new();
        assert!(PythonHooks.synthesize_members("Foo", &[], &arena).is_empty());
    }

    #[test]
    fn static_instance_send_sync() {
        fn require<T: Send + Sync + ?Sized>() {}
        require::<PythonHooks>();
    }
}
