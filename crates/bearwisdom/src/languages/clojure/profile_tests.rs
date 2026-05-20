use super::CLOJURE_PROFILE;
use crate::type_checker::profile::language_profile::DispatchAxis;

#[test]
fn clojure_profile_identity() {
    assert_eq!(CLOJURE_PROFILE.id, "clojure");
    assert_eq!(CLOJURE_PROFILE.qname_separator, "/");
}

#[test]
fn clojure_dispatch_axis_is_multi_arg() {
    assert_eq!(CLOJURE_PROFILE.dispatch_axis, DispatchAxis::MultiArg);
}

#[test]
fn clojure_profile_engine_primary_disabled() {
    assert!(!CLOJURE_PROFILE.engine_primary);
}
