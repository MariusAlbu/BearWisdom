// Minimal LanguageProfile for Bicep in shadow mode.

use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

// Bicep user-defined functions and types; ARM resource API methods land as
// methods. `variable`/`function` are valid TypeRef targets (`param x type` and
// user-defined functions referenced as types).
const BICEP_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Method,
            SymbolKind::Function,
            SymbolKind::Constructor,
        ],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
            SymbolKind::Variable,
            SymbolKind::Function,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Function],
    ),
];

pub const BICEP_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    // The `bicep-runtime` ecosystem stub (ext:bicep-runtime:namespace.bicep)
    // qualifies every ARM function under `bicep.builtins` and every decorator
    // under `bicep.decorators`. Bicep has no import statements — both sets
    // are implicitly in scope for every file — so a bare `uniqueString(...)`
    // or `@description(...)` binds as a direct member of one of these two
    // namespaces via `ImplicitPreludeRule`.
    implicit_prelude_namespaces: &["bicep.builtins", "bicep.decorators"],
    compiled_name_prefixes: &[],
    id: "bicep",
    qname_separator: ".",
    declaration_merging: crate::type_checker::profile::language_profile::MergeScope::None,
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: false,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: &[],
    kind_compatible_table: BICEP_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: Some(super::predicates::is_azure_resource_type),
    namespace_decline: None,
    imports: crate::type_checker::profile::language_profile::ImportAxes {
        // `sys`/`az` are namespace aliases over the bicep-runtime ambient symbols
        // (both members land under `bicep.builtins`/`bicep.decorators`), not qname
        // path segments. Strip the alias so `sys.concat`/`az.resourceId` resolve
        // against the bare ambient symbol.
        decline_qualified_when_prefix_imported: false,
        import_resolution: None,
        import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
        module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
        module_anchor_terminal: false,
        relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
        external_by_import: None,
        module_scope: crate::type_checker::profile::language_profile::ModuleScope::Off,
        wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
        namespace_imports_are_wildcards: false,
        ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
        head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
        file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
        alias_module_qname: false,
        module_prefix_rewrites:
            crate::type_checker::profile::language_profile::ModulePrefixRewrites::Off,
        workspace_packages: false,
        reexport_barrel_stems: &["index"],
        self_package_root: None,
        wildcard_workspace_scope: false,
    },
    module_skip: None,
    ambient_namespace_prefixes: &["sys", "az"],
    // The `az` namespace registers `list*` as a regex overload, not a finite
    // name set; arbitrary `listFoo()` / `listConnectionStrings()` calls fold to
    // the vendored `list` builtin (the family base shipped in the namespace
    // surface). Anchored `list` + uppercase, so `list`/`listener`/`listing`
    // don't fold.
    wildcard_builtins: &[crate::type_checker::profile::language_profile::WildcardBuiltin {
        prefix: "list",
        fold_to: "list",
    }],
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    delegate_wrappers: &[],
    overload_pick_all: false,
    argument_dependent_lookup: false,
    associated_type_projection: false,
    blanket_impl_resolution: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::Off,
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    namespaceless_global_type_lookup:
        crate::type_checker::profile::language_profile::NamespaceScope::Off,
    explicit_member_import: false,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["//"],
    visibility_keywords: &[],
    function_prototype_types: &[],
    external_contract_reduction: true,
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
