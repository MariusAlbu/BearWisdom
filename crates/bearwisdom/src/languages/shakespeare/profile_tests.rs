use super::{CASSIUS_PROFILE, HAMLET_PROFILE, JULIUS_PROFILE, LUCIUS_PROFILE};

#[test]
fn shakespeare_profile_ids_and_shadow_mode() {
    assert_eq!(HAMLET_PROFILE.id, "hamlet");
    assert_eq!(CASSIUS_PROFILE.id, "cassius");
    assert_eq!(LUCIUS_PROFILE.id, "lucius");
    assert_eq!(JULIUS_PROFILE.id, "julius");
    assert!(!HAMLET_PROFILE.engine_primary);
    assert!(!CASSIUS_PROFILE.engine_primary);
    assert!(!LUCIUS_PROFILE.engine_primary);
    assert!(!JULIUS_PROFILE.engine_primary);
}
