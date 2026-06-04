use super::TWIG_PROFILE;

#[test]
fn twig_profile_identity_and_shadow_mode() {
    assert_eq!(TWIG_PROFILE.id, "twig");
}
