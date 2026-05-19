// =============================================================================
// languages/python/hooks.rs — PythonHooks impl of LanguageEngineHooks.
// =============================================================================

use crate::type_checker::profile::hooks::LanguageEngineHooks;

pub struct PythonHooks;

impl LanguageEngineHooks for PythonHooks {}

pub static PYTHON_HOOKS: PythonHooks = PythonHooks;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_instance_send_sync() {
        fn require<T: Send + Sync + ?Sized>() {}
        require::<PythonHooks>();
    }
}
