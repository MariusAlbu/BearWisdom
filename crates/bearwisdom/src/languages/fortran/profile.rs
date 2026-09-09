// LanguageProfile for Fortran in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const FORTRAN_KIND_TABLE: KindTable = &[
    (EdgeKind::Calls, &[SymbolKind::Function, SymbolKind::Method]),
    (EdgeKind::TypeRef, &[SymbolKind::Struct, SymbolKind::Module]),
    // An EXTENDS clause's base is another derived type, extracted as Struct.
    (EdgeKind::Inherits, &[SymbolKind::Struct]),
];

const FORTRAN_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("integer", PrimKind::Int),
    ("real", PrimKind::Float),
    ("double precision", PrimKind::Float),
    ("complex", PrimKind::Float),
    ("logical", PrimKind::Bool),
    ("character", PrimKind::Str),
];

pub const FORTRAN_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    implicit_prelude_namespaces: &[],
    compiled_name_prefixes: &[],
    id: "fortran",
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
    reference_member_projection: false,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: FORTRAN_PRIMITIVES,
    kind_compatible_table: FORTRAN_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: Some(super::predicates::is_fortran_intrinsic),
    namespace_decline: None,
    imports: crate::type_checker::profile::language_profile::ImportAxes {
        decline_qualified_when_prefix_imported: false,
        import_resolution: None,
        import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
        module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::On(
            crate::type_checker::profile::language_profile::ModuleAnchorBind::MemberOfModuleType,
        ),
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
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::Spec(
        crate::type_checker::profile::language_profile::NormSpec {
            case_insensitive: true,
            strip_chars: &[],
            strip_prefixes: &[],
            strip_sigils: &[],
        },
    ),
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
        crate::type_checker::profile::language_profile::NamespaceScope::Global,
    explicit_member_import: false,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["!"],
    visibility_keywords: &[],
    function_prototype_types: &[],
    external_contract_reduction: true,
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
