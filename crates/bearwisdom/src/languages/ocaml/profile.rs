// =============================================================================
// languages/ocaml/profile.rs — LanguageProfile for OCaml.
//
// Registered in shadow mode. OCaml's module-level dispatch and structural
// typing on records / variants need hook coverage before engine takeover.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const OCAML_KIND_TABLE: KindTable = &[
    // Variant constructors (`Some x`, `Ok value`) are applied like functions
    // and extracted as `Struct`; accept that shape for `Calls` so a bare
    // constructor application binds to its declaration.
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
            SymbolKind::Struct,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Module]),
    (
        EdgeKind::Implements,
        &[SymbolKind::Module, SymbolKind::Interface],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::Struct,
            SymbolKind::TypeAlias,
            SymbolKind::Module,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Constructor],
    ),
];

const OCAML_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("int", PrimKind::Int),
    ("float", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("string", PrimKind::Str),
    ("char", PrimKind::Str),
    ("unit", PrimKind::Unit),
];

pub const OCAML_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    implicit_prelude_namespaces: &[],
    compiled_name_prefixes: &[],
    id: "ocaml",
    qname_separator: ".",
    declaration_merging: crate::type_checker::profile::language_profile::MergeScope::None,
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Structural,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Lwt.t", "Async.Deferred.t"],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: OCAML_PRIMITIVES,
    kind_compatible_table: OCAML_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    imports: crate::type_checker::profile::language_profile::ImportAxes {
        decline_qualified_when_prefix_imported: false,
        import_resolution: None,
        import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
        module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::On(
            crate::type_checker::profile::language_profile::ModuleAnchorBind::ByFileStem {
                against: crate::type_checker::profile::language_profile::StemSource::ModuleLeaf,
            },
        ),
        module_anchor_terminal: false,
        relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
        external_by_import: None,
        module_scope: crate::type_checker::profile::language_profile::ModuleScope::Off,
        wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::FileStem {
            underscore_prefix: false,
        },
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
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["(**"],
    visibility_keywords: &[],
    function_prototype_types: &[],
    external_contract_reduction: true,
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
