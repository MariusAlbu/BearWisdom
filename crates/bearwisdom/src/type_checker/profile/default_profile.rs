// =============================================================================
// type_checker/profile/default_profile.rs — the conservative fallback profile
//
// The registry hands out `&DEFAULT_PROFILE` for any language id without a
// bespoke profile. Every axis holds the safe per-axis "Default" entry from
// research/architecture/03-language-profile-spec.html.
// =============================================================================

use super::language_profile::*;

/// Conservative defaults applied to languages without a bespoke profile.
/// Every value is the safe fallback per doc 3's per-axis "Default" entry.
pub const DEFAULT_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    implicit_prelude_namespaces: &[],
    compiled_name_prefixes: &[],
    id: "default",
    qname_separator: ".",
    declaration_merging: crate::type_checker::profile::language_profile::MergeScope::None,
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: AncestorOrder::Bfs,
    members_can_be_external: false,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    function_prototype_types: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: &[],
    kind_compatible_table: PERMISSIVE_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    imports: crate::type_checker::profile::language_profile::ImportAxes {
        decline_qualified_when_prefix_imported: false,
        import_resolution: None,
        import_module_path: ImportModulePath::None,
        module_anchor: ModuleAnchor::Off,
        module_anchor_terminal: false,
        relative_marker: RelativeMarker::None,
        external_by_import: None,
        module_scope: ModuleScope::Off,
        wildcard_match: WildcardMatch::QnameUnder,
        namespace_imports_are_wildcards: false,
        ext_match: ExtMatch::PkgSegment,
        head_alias: HeadAliasBind::Off,
        file_scoped_imports: FileScopedImports::Off,
        alias_module_qname: false,
        module_prefix_rewrites: ModulePrefixRewrites::Off,
        workspace_packages: false,
        reexport_barrel_stems: &["index"],
        self_package_root: None,
        wildcard_workspace_scope: false,
    },
    module_skip: None,
    ambient_namespace_prefixes: &[],
    wildcard_builtins: &[],
    name_normalization: NameNormalization::None,
    delegate_wrappers: &[],
    overload_pick_all: false,
    argument_dependent_lookup: false,
    associated_type_projection: false,
    blanket_impl_resolution: false,
    ambient_globals: AmbientGlobals::Off,
    namespaceless_global_type_lookup: NamespaceScope::Off,
    explicit_member_import: false,
    self_receiver_discovery: SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[ConstructorPattern::CallableClass],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &[],
    visibility_keywords: &[],
    external_contract_reduction: true,
};

#[cfg(test)]
#[path = "default_profile_tests.rs"]
mod tests;
