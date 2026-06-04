use super::HCL_PROFILE;
use crate::type_checker::profile::language_profile::HeadAliasBind;

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
        HCL_PROFILE.head_alias,
        HeadAliasBind::OnSameFile {
            require_kind: Some("class"),
        }
    );
}
