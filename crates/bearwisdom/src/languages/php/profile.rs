// =============================================================================
// languages/php/profile.rs — LanguageProfile for PHP.
//
// Engine-side type-system data for PHP.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable,
    LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const PHP_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
        ],
    ),
    (
        EdgeKind::Inherits,
        &[SymbolKind::Class, SymbolKind::Interface],
    ),
    (EdgeKind::Implements, &[SymbolKind::Interface]),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::Trait,
        ],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class]),
];

const PHP_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("string", PrimKind::Str),
    ("int", PrimKind::Int),
    ("integer", PrimKind::Int),
    ("float", PrimKind::Float),
    ("double", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("boolean", PrimKind::Bool),
    ("void", PrimKind::Unit),
    ("null", PrimKind::Unit),
    ("mixed", PrimKind::Unknown),
    ("never", PrimKind::Never),
];

/// PHP profile.
pub const PHP_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    implicit_prelude_namespaces: &[],
    compiled_name_prefixes: &[],
    id: "php",
    qname_separator: "\\",
    declaration_merging: crate::type_checker::profile::language_profile::MergeScope::None,
    self_keywords: &["$this", "self", "static", "parent"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: true,
    look_through_optional: true,
    reference_member_projection: false,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: PHP_PRIMITIVES,
    kind_compatible_table: PHP_KIND_TABLE,
    // Same-namespace + `use`-statement qualification of a bare receiver type
    // via the structured walker's `qualify_current_ty`.
    chain_qualification: ChainQualification::SamePackageAndImports,
    builtin_skip: Some(super::predicates::is_php_builtin),
    namespace_decline: None,
    imports: crate::type_checker::profile::language_profile::ImportAxes {
        decline_qualified_when_prefix_imported: false,
        import_resolution: None,
        import_module_path:
            crate::type_checker::profile::language_profile::ImportModulePath::FromModuleField,
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
    ambient_namespace_prefixes: &[],
    wildcard_builtins: &[],
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
    constructor_patterns: &[ConstructorPattern::New],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AttrBracket),
    doc_comment_kinds: &["/**"],
    visibility_keywords: &[
        ("public", Visibility::Public),
        ("private", Visibility::Private),
        ("protected", Visibility::Protected),
    ],
    function_prototype_types: &[],
    external_contract_reduction: true,
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
