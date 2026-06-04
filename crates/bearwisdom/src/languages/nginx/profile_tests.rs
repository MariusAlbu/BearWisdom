use super::NGINX_PROFILE;

#[test]
fn nginx_profile_identity_and_shadow_mode() {
    assert_eq!(NGINX_PROFILE.id, "nginx");
}
