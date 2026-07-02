use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, LanguageProfile, NameTransform, SelectorResolution,
    SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};
use crate::types::EdgeKind;

pub const ANGULAR_TEMPLATE_PROFILE: LanguageProfile = LanguageProfile {
    id: "angular_template",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: false,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
    single_inner_wrappers: &[],
    deref_wrapper: None,
    iterator_method: None,
    // A `.component.html` embedded region carries a synthetic `let <#ref>: any;`
    // prelude (TypeScript). The resolve loop selects this host profile by file
    // language, so the embedded TS primitive surface must be recognized here to drop
    // those keyword TypeRefs from the unresolved count, not record them as references.
    primitive_mapping: crate::languages::typescript::profile::TS_PRIMITIVES,
    kind_compatible_table: PERMISSIVE_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    wildcard_builtins: &[],
    import_resolution: None,
    import_module_path:
        crate::type_checker::profile::language_profile::ImportModulePath::FromModuleField,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::On(
        crate::type_checker::profile::language_profile::ModuleAnchorBind::NameExactKind,
    ),
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::DotSlashPrefix,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    module_scope: crate::type_checker::profile::language_profile::ModuleScope::Off,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
    file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
    alias_module_qname: false,
    module_prefix_rewrites:
        crate::type_checker::profile::language_profile::ModulePrefixRewrites::On {
            definitely_typed: true,
            deep_import_peel: true,
            decline_bare_directory_match: true,
        },
    workspace_packages: true,
    reexport_barrel_stems: &["index"],
    self_package_root: None,
    wildcard_workspace_scope: false,
    overload_pick_all: true,
    argument_dependent_lookup: false,
    associated_type_projection: false,
    blanket_impl_resolution: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::On {
        instantiate_accepts_variable: true,
    },
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    // Template component-tag / attribute-directive refs bind to the decorated
    // class via the selector map; the raw target, then its kebab form.
    selector_resolution: Some(SelectorResolution {
        edge_kinds: &[EdgeKind::Calls],
        name_transforms: &[NameTransform::PascalToKebab],
    }),
    namespaceless_global_type_lookup:
        crate::type_checker::profile::language_profile::NamespaceScope::Off,
    explicit_member_import: false,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["<!--"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
