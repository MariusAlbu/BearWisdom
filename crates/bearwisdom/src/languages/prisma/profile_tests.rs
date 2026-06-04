use super::PRISMA_PROFILE;

#[test]
fn prisma_profile_identity_and_shadow_mode() {
    assert_eq!(PRISMA_PROFILE.id, "prisma");
}
