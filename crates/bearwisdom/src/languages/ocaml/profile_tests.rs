use super::OCAML_PROFILE;
use crate::type_checker::profile::language_profile::SupertypeDiscovery;

#[test]
fn ocaml_profile_identity() {
    assert_eq!(OCAML_PROFILE.id, "ocaml");
    assert!(OCAML_PROFILE.self_keywords.is_empty());
}

#[test]
fn ocaml_supertype_discovery_is_structural() {
    assert_eq!(
        OCAML_PROFILE.supertype_discovery,
        SupertypeDiscovery::Structural
    );
}

#[test]
fn ocaml_async_wrappers_cover_lwt_and_async() {
    assert!(OCAML_PROFILE.async_wrappers.contains(&"Lwt.t"));
    assert!(OCAML_PROFILE.async_wrappers.contains(&"Async.Deferred.t"));
}
