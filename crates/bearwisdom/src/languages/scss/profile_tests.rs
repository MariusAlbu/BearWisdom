use super::SCSS_PROFILE;

#[test]
fn scss_profile_identity_and_shadow_mode() {
    assert_eq!(SCSS_PROFILE.id, "scss");
}

#[test]
fn scss_module_skip_declines_sass_builtin_and_css_hint() {
    // The two former resolve_ref module declines (Sass built-in `@use`, the
    // synthesized css() hint) are now the profile's module-keyed pre-ladder
    // decline.
    let skip = SCSS_PROFILE.module_skip.expect("scss sets module_skip");
    assert!(skip("sass:math"));
    assert!(skip(crate::languages::scss::extract::SCSS_CSS_FN_HINT));
    assert!(!skip("./vars"));
}
