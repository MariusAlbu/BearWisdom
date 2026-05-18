use super::*;
use crate::types::EdgeKind;

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
fn permissive_kind_table_accepts_all_kinds() {
    assert!(KindCompatibility::check(
        PERMISSIVE_KIND_TABLE,
        EdgeKind::Calls,
        SymbolKind::Method,
    ));
    assert!(KindCompatibility::check(
        PERMISSIVE_KIND_TABLE,
        EdgeKind::Inherits,
        SymbolKind::Variable,
    ));
}

#[test]
fn kind_compat_table_respects_declared_kinds() {
    static TABLE: &[(EdgeKind, &[SymbolKind])] = &[
        (
            EdgeKind::Calls,
            &[SymbolKind::Method, SymbolKind::Function, SymbolKind::Constructor],
        ),
        (EdgeKind::Inherits, &[SymbolKind::Class]),
    ];
    assert!(KindCompatibility::check(TABLE, EdgeKind::Calls, SymbolKind::Method));
    assert!(!KindCompatibility::check(TABLE, EdgeKind::Calls, SymbolKind::Variable));
    assert!(KindCompatibility::check(TABLE, EdgeKind::Inherits, SymbolKind::Class));
    assert!(!KindCompatibility::check(TABLE, EdgeKind::Inherits, SymbolKind::Interface));
}

#[test]
fn kind_compat_table_defaults_unlisted_edge_kinds_to_permissive() {
    static TABLE: &[(EdgeKind, &[SymbolKind])] =
        &[(EdgeKind::Calls, &[SymbolKind::Method])];
    // TypeRef is not in the table → defaults to "any kind accepted".
    assert!(KindCompatibility::check(
        TABLE,
        EdgeKind::TypeRef,
        SymbolKind::Variable,
    ));
}

#[test]
fn method_bucket_construction_is_const_friendly() {
    static BUCKET: MethodBucket = MethodBucket {
        arg: ArgKey::Named("public"),
        container: BucketContainer::ListCall("list"),
        member_shape: MemberShape::NameEqFunction,
        visibility: Visibility::Public,
    };
    assert_eq!(BUCKET.arg, ArgKey::Named("public"));
    assert_eq!(BUCKET.visibility, Visibility::Public);
}

#[test]
fn class_builder_spec_is_const_friendly() {
    static BUCKETS: &[MethodBucket] = &[MethodBucket {
        arg: ArgKey::Named("public"),
        container: BucketContainer::ListCall("list"),
        member_shape: MemberShape::NameEqFunction,
        visibility: Visibility::Public,
    }];
    static SPEC: ClassBuilderSpec = ClassBuilderSpec {
        callee: "R6Class",
        accepted_namespaces: &["R6"],
        class_name_source: ClassNameSource::LhsThenArg(0),
        method_buckets: BUCKETS,
        inherits_arg: Some(ArgKey::Named("inherit")),
    };
    assert_eq!(SPEC.callee, "R6Class");
    assert_eq!(SPEC.method_buckets.len(), 1);
    assert!(matches!(
        SPEC.class_name_source,
        ClassNameSource::LhsThenArg(0)
    ));
}
