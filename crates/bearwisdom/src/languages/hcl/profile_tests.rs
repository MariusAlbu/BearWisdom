// =============================================================================
// hcl/profile_tests.rs — profile-axis and full-ladder bind tests.
//
// Terraform `var`/`local`/resource names are module-flat: every `.tf` in a
// directory shares one namespace, so a `var.X` ref binds cross-file. The
// `var`/`local` sigil head is a self-keyword the bare-name rung strips, then
// `namespaceless_global_type_lookup == Global` first-match-binds the project's
// own `X`; a same-named ext: stub declines and stays external.
// =============================================================================

use super::HCL_PROFILE;
use crate::type_checker::profile::language_profile::{HeadAliasBind, NamespaceScope};

#[test]
fn hcl_profile_carries_resolution_data() {
    assert_eq!(HCL_PROFILE.id, "hcl");
    // `var.X` / `local.X` heads are stripped by the bare-name probes.
    assert_eq!(HCL_PROFILE.self_keywords, &["var", "local"]);
    // Terraform meta-references decline before the ladder.
    let skip = HCL_PROFILE.builtin_skip.expect("builtin_skip set");
    assert!(skip("each.value"));
    assert!(skip("count.index"));
    assert!(!skip("aws_instance.web"));
    // Provider-alias heads bind to an in-file `provider` class.
    assert_eq!(
        HCL_PROFILE.imports.head_alias,
        HeadAliasBind::OnSameFile {
            require_kind: Some("class"),
        }
    );
}

#[test]
fn hcl_namespaceless_global_is_on() {
    // Terraform `var`/`local`/resource names are module-flat, so a `var.X` ref
    // binds via the dead-last first-match-by-name rung.
    assert_eq!(
        HCL_PROFILE.namespaceless_global_type_lookup,
        NamespaceScope::Global
    );
}

