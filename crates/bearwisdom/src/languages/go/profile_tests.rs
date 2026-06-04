use super::*;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, SupertypeDiscovery,
};

#[test]
fn id_matches() {
    assert_eq!(GO_PROFILE.id, "go");
}

#[test]
fn chain_qualification_is_package_short_name() {
    // Members keyed under the import's package short name (`gin.NewRouter`).
    assert_eq!(
        GO_PROFILE.chain_qualification,
        ChainQualification::PackageShortName
    );
}

#[test]
fn structural_discovery_is_set() {
    // Go's defining characteristic.
    assert_eq!(GO_PROFILE.supertype_discovery, SupertypeDiscovery::Structural);
    assert_eq!(GO_PROFILE.dispatch_axis, DispatchAxis::Receiver);
    assert!(GO_PROFILE.has_generics);
}

#[test]
fn no_self_keywords_or_decorator_syntax() {
    assert!(GO_PROFILE.self_keywords.is_empty());
    assert!(GO_PROFILE.decorator_syntax.is_none());
}

#[test]
fn no_async_wrappers() {
    // Goroutines + channels, not value wrappers.
    assert!(GO_PROFILE.async_wrappers.is_empty());
}

#[test]
fn primitives_include_go_builtins() {
    let names: Vec<&str> = GO_PROFILE
        .primitive_mapping
        .iter()
        .map(|(n, _)| *n)
        .collect();
    for canonical in ["string", "int", "int64", "bool", "float64", "byte", "rune"] {
        assert!(names.contains(&canonical), "missing Go primitive: {canonical}");
    }
}
