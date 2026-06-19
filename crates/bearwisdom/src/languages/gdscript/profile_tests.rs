// =============================================================================
// gdscript/profile_tests.rs — profile-axis and full-ladder bind tests.
//
// GDScript `class_name`-registered scripts form one flat global namespace, so a
// bare cross-file `extends` / type / call ref binds to its sibling-file
// declaration. `namespaceless_global_type_lookup == Global` drives that bind
// via the dead-last first-match-by-name rung; a same-named ext: engine stub
// declines and stays external.
// =============================================================================

use super::GDSCRIPT_PROFILE;
use crate::type_checker::profile::language_profile::NamespaceScope;

#[test]
fn gdscript_profile_identity_and_shadow_mode() {
    assert_eq!(GDSCRIPT_PROFILE.id, "gdscript");
}

#[test]
fn gdscript_namespaceless_global_is_on() {
    // `class_name`-registered scripts are flat-global, so a bare cross-file ref
    // binds via the dead-last first-match-by-name rung.
    assert_eq!(
        GDSCRIPT_PROFILE.namespaceless_global_type_lookup,
        NamespaceScope::Global
    );
}

