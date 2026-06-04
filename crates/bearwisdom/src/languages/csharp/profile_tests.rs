use super::*;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, SupertypeDiscovery,
};
use crate::types::SymbolKind;

#[test]
fn id_matches() {
    assert_eq!(CSHARP_PROFILE.id, "csharp");
}

#[test]
fn chain_qualification_is_same_package_and_imports() {
    // Same-namespace + using-directive qualification through the engine walker.
    assert_eq!(
        CSHARP_PROFILE.chain_qualification,
        ChainQualification::SamePackageAndImports
    );
}

#[test]
fn structural_choices() {
    assert_eq!(CSHARP_PROFILE.supertype_discovery, SupertypeDiscovery::Explicit);
    assert_eq!(CSHARP_PROFILE.dispatch_axis, DispatchAxis::Receiver);
    assert!(CSHARP_PROFILE.has_generics);
    // Nullable<T> is a wrapper type with explicit unwrap; engine must not
    // peel transparently.
    assert!(!CSHARP_PROFILE.look_through_optional);
    // Extension methods + NuGet externals.
    assert!(CSHARP_PROFILE.members_can_be_external);
}

#[test]
fn this_and_base_are_self_keywords() {
    assert!(CSHARP_PROFILE.self_keywords.contains(&"this"));
    assert!(CSHARP_PROFILE.self_keywords.contains(&"base"));
}

#[test]
fn task_and_valuetask_are_async_wrappers() {
    assert!(CSHARP_PROFILE.async_wrappers.contains(&"Task"));
    assert!(CSHARP_PROFILE.async_wrappers.contains(&"ValueTask"));
}

#[test]
fn delegate_appears_in_calls_and_typeref_tables() {
    use crate::type_checker::profile::language_profile::KindCompatibility;
    use crate::types::EdgeKind;
    let t = CSHARP_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Delegate));
    assert!(KindCompatibility::check(t, EdgeKind::TypeRef, SymbolKind::Delegate));
}
