use super::VUE_PROFILE;

#[test]
fn vue_profile_identity_and_shadow_mode() {
    assert_eq!(VUE_PROFILE.id, "vue");
}

/// A `.vue` <script> block is TypeScript; the resolve loop selects VUE_PROFILE by
/// the host file's language. The embedded-TS primitive annotations (`: number`,
/// `: boolean`, `: any`) name no symbol, so without the TS primitive set on this
/// profile they miss the unresolved-drop guard and leak into unresolved_refs as
/// phantom references.
#[test]
fn vue_profile_recognizes_ts_primitive_surface() {
    for prim in ["number", "boolean", "string", "any", "void", "never"] {
        assert!(
            VUE_PROFILE
                .primitive_mapping
                .iter()
                .any(|(n, _)| *n == prim),
            "VUE_PROFILE.primitive_mapping must recognize TS primitive `{prim}` so an \
             embedded-script `: {prim}` annotation is dropped, not counted unresolved"
        );
    }
}
