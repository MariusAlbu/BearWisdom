use super::DEFAULT_PROFILE;
use crate::type_checker::profile::{ConstructorPattern, DispatchAxis, SupertypeDiscovery};

#[test]
fn default_profile_uses_dot_separator_and_receiver_dispatch() {
    assert_eq!(DEFAULT_PROFILE.qname_separator, ".");
    assert_eq!(DEFAULT_PROFILE.dispatch_axis, DispatchAxis::Receiver);
    assert_eq!(
        DEFAULT_PROFILE.supertype_discovery,
        SupertypeDiscovery::Explicit
    );
    assert!(!DEFAULT_PROFILE.has_generics);
    assert!(!DEFAULT_PROFILE.has_sum_types);
}

#[test]
fn default_profile_treats_class_as_callable_constructor() {
    assert_eq!(
        DEFAULT_PROFILE.constructor_patterns,
        &[ConstructorPattern::CallableClass]
    );
}

#[test]
fn default_profile_declares_no_wildcard_builtins() {
    assert!(DEFAULT_PROFILE.wildcard_builtins.is_empty());
}
