use super::{QUARTO_PROFILE, RMARKDOWN_PROFILE};

#[test]
fn rmarkdown_profile_identity_and_shadow_mode() {
    assert_eq!(RMARKDOWN_PROFILE.id, "rmarkdown");
    assert!(!RMARKDOWN_PROFILE.engine_primary);
}

#[test]
fn quarto_profile_identity_and_shadow_mode() {
    assert_eq!(QUARTO_PROFILE.id, "quarto");
    assert!(!QUARTO_PROFILE.engine_primary);
}
