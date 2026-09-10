use super::*;
use crate::types::EdgeKind;

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
            &[
                SymbolKind::Method,
                SymbolKind::Function,
                SymbolKind::Constructor,
            ],
        ),
        (EdgeKind::Inherits, &[SymbolKind::Class]),
    ];
    assert!(KindCompatibility::check(
        TABLE,
        EdgeKind::Calls,
        SymbolKind::Method
    ));
    assert!(!KindCompatibility::check(
        TABLE,
        EdgeKind::Calls,
        SymbolKind::Variable
    ));
    assert!(KindCompatibility::check(
        TABLE,
        EdgeKind::Inherits,
        SymbolKind::Class
    ));
    assert!(!KindCompatibility::check(
        TABLE,
        EdgeKind::Inherits,
        SymbolKind::Interface
    ));
}

#[test]
fn kind_compat_table_defaults_unlisted_edge_kinds_to_permissive() {
    static TABLE: &[(EdgeKind, &[SymbolKind])] = &[(EdgeKind::Calls, &[SymbolKind::Method])];
    // TypeRef is not in the table → defaults to "any kind accepted".
    assert!(KindCompatibility::check(
        TABLE,
        EdgeKind::TypeRef,
        SymbolKind::Variable,
    ));
}

#[test]
fn wildcard_builtin_folds_only_on_uppercase_anchor() {
    static WB: WildcardBuiltin = WildcardBuiltin {
        prefix: "list",
        fold_to: "list",
    };
    // `prefix` + uppercase → folds to the family base.
    assert_eq!(WB.fold("listKeys"), Some("list"));
    assert_eq!(WB.fold("listConnectionStrings"), Some("list"));
    assert_eq!(WB.fold("listFoo"), Some("list"));
    // Bare prefix, lowercase continuation, or unrelated word → no fold.
    assert_eq!(WB.fold("list"), None);
    assert_eq!(WB.fold("listener"), None);
    assert_eq!(WB.fold("listing"), None);
    assert_eq!(WB.fold("resourceId"), None);
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
#[test]
fn source_qname_conversion_uses_the_profile_separator() {
    let mut profile = super::DEFAULT_PROFILE;
    profile.qname_separator = "::";
    assert_eq!(profile.simple_name("crate::net::Client"), "Client");
    assert_eq!(
        profile.index_qname_join("crate::net", "Client"),
        "crate.net.Client"
    );
    assert_eq!(
        profile.index_qname_from_source("crate.net.Client"),
        "crate.net.Client"
    );
    assert_eq!(
        profile.index_qname_path_from_source("crate::net::Client"),
        "crate/net/Client"
    );
}

#[test]
fn member_surface_adapters_are_opt_in_and_fail_closed() {
    assert_eq!(DEFAULT_PROFILE.primitive_member_head("string"), None);
    assert!(!DEFAULT_PROFILE.has_homogeneous_computed_access("Array"));

    let ts = &crate::languages::typescript::TYPESCRIPT_PROFILE;
    assert_eq!(
        ts.primitive_member_head("string").as_deref(),
        Some("String")
    );
    assert!(ts.has_homogeneous_computed_access("ReadonlyArray"));

    let rust = &crate::languages::rust_lang::RUST_PROFILE;
    assert!(rust.has_homogeneous_computed_access("Vec"));
    assert!(!rust.has_homogeneous_computed_access("Array"));
}
