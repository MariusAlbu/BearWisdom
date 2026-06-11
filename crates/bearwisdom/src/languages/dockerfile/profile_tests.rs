use super::DOCKERFILE_PROFILE;

#[test]
fn dockerfile_profile_identity_and_shadow_mode() {
    assert_eq!(DOCKERFILE_PROFILE.id, "dockerfile");
}

#[test]
fn dockerfile_registry_image_modules_decline() {
    let skip = DOCKERFILE_PROFILE
        .module_skip
        .expect("dockerfile declines registry-image FROM modules");
    // Tagged / registry-qualified images are published images, not stages.
    assert!(skip("ubuntu:latest"));
    assert!(skip("node:18-alpine"));
    assert!(skip("docker.io/library/node"));
    assert!(skip("gcr.io/distroless/static:nonroot"));
    // A bare untagged image / multi-stage reuse name stays on the ladder.
    assert!(!skip("builder"));
    assert!(!skip("node"));
}
