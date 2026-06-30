use super::ANGULAR_TEMPLATE_PROFILE;

#[test]
fn angular_template_profile_identity_and_shadow_mode() {
    assert_eq!(ANGULAR_TEMPLATE_PROFILE.id, "angular_template");
}

/// A `.component.html` embedded region carries a synthetic `let <#ref>: any;`
/// prelude (TypeScript). The resolve loop selects ANGULAR_TEMPLATE_PROFILE by the
/// host file's language, so the embedded TS primitive surface must be recognized
/// here or the prelude's `: any` annotations leak as unresolved references.
#[test]
fn angular_template_profile_recognizes_ts_primitive_surface() {
    for prim in ["any", "number", "boolean", "void", "never"] {
        assert!(
            ANGULAR_TEMPLATE_PROFILE
                .primitive_mapping
                .iter()
                .any(|(n, _)| *n == prim),
            "ANGULAR_TEMPLATE_PROFILE.primitive_mapping must recognize TS primitive \
             `{prim}`: the synthetic `let <#ref>: {prim};` template prelude must not \
             leak as an unresolved ref"
        );
    }
}
