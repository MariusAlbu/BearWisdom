use super::*;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, LanguageProfile, SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};

static TS_PROFILE: LanguageProfile = LanguageProfile {
    id: "typescript",
    qname_separator: ".",
    self_keywords: &["this", "super"],
    supertype_discovery: SupertypeDiscovery::Both,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: true,
    async_wrappers: &["Promise", "PromiseLike", "Thenable"],
    container_accessors: &[],
    single_inner_wrappers: &[],
    deref_wrapper: None,
    iterator_method: Some("[Symbol.iterator]"),
    primitive_mapping: &[],
    kind_compatible_table: PERMISSIVE_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    wildcard_builtins: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    module_scope: crate::type_checker::profile::language_profile::ModuleScope::Off,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
    file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
    alias_module_qname: false,
    module_prefix_rewrites:
        crate::type_checker::profile::language_profile::ModulePrefixRewrites::Off,
    workspace_packages: false,
    overload_pick_all: false,
    argument_dependent_lookup: false,
    associated_type_projection: false,
    blanket_impl_resolution: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::Off,
    namespaceless_global_type_lookup:
        crate::type_checker::profile::language_profile::NamespaceScope::Off,
    explicit_member_import: false,
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &[],
    visibility_keywords: &[],
};

#[test]
fn lookup_unregistered_id_returns_default_profile() {
    let registry = ProfileRegistry::new();
    let p = registry.get("python");
    assert_eq!(p.id, "default");
}

#[test]
fn register_then_lookup_returns_registered_profile() {
    let mut registry = ProfileRegistry::new();
    registry.register(&TS_PROFILE);
    let p = registry.get("typescript");
    assert_eq!(p.id, "typescript");
    assert!(p.has_generics);
    assert_eq!(p.supertype_discovery, SupertypeDiscovery::Both);
}

#[test]
fn contains_returns_false_for_unregistered_id() {
    let registry = ProfileRegistry::new();
    assert!(!registry.contains("python"));
}

#[test]
fn contains_returns_true_after_registration() {
    let mut registry = ProfileRegistry::new();
    registry.register(&TS_PROFILE);
    assert!(registry.contains("typescript"));
    assert!(!registry.contains("python"));
}

#[test]
fn register_overwrites_previous_profile_for_same_id() {
    let mut registry = ProfileRegistry::new();
    registry.register(&TS_PROFILE);
    registry.register(&TS_PROFILE);
    assert_eq!(registry.len(), 1);
}
