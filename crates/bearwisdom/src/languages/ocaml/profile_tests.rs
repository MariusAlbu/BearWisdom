use super::{OCAML_KIND_TABLE, OCAML_PROFILE};
use crate::type_checker::profile::language_profile::{KindCompatibility, SupertypeDiscovery};
use crate::types::{EdgeKind, SymbolKind};

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

#[test]
fn variant_constructor_application_is_callable() {
    // `Some x` / `Ok value`: a variant constructor extracted as `Struct`,
    // applied as a `Calls` ref, must be a compatible target.
    assert!(KindCompatibility::check(
        OCAML_KIND_TABLE,
        EdgeKind::Calls,
        SymbolKind::Struct,
    ));
}

#[test]
fn ordinary_function_call_unaffected() {
    assert!(KindCompatibility::check(
        OCAML_KIND_TABLE,
        EdgeKind::Calls,
        SymbolKind::Function,
    ));
}
