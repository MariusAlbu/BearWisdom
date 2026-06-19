// =============================================================================
// jinja/profile_tests.rs — profile-axis and full-ladder bind tests.
//
// Jinja template variables resolve against sibling var sources (e.g. Ansible
// `defaults`/`vars`) in one flat namespace, not a per-file scope. A bare
// `{{ var }}` ref binds to its sibling-file declaration via the dead-last
// first-match-by-name rung (`namespaceless_global_type_lookup == Global`); a
// same-named ext: stub declines and stays external. An `Imports` template-path
// ref takes the import-path strategy first and never reaches this rung.
// =============================================================================

use super::JINJA_PROFILE;
use crate::type_checker::profile::language_profile::NamespaceScope;

#[test]
fn jinja_profile_identity_and_shadow_mode() {
    assert_eq!(JINJA_PROFILE.id, "jinja");
}

#[test]
fn jinja_namespaceless_global_is_on() {
    // Template variables are flat across sibling var sources, so a bare var ref
    // binds via the dead-last first-match-by-name rung.
    assert_eq!(
        JINJA_PROFILE.namespaceless_global_type_lookup,
        NamespaceScope::Global
    );
}

