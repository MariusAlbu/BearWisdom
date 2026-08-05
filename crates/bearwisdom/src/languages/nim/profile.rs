// LanguageProfile for Nim in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const NIM_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        // Object construction `Foo(field: x)` and distinct/type conversion
        // `Slot(x)` are call-syntactic in Nim and bind to the type declaration.
        // An enum value `Color.Red` / a pure-enum member used call-syntactically
        // binds to the EnumMember.
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
            SymbolKind::TypeAlias,
            SymbolKind::Struct,
            SymbolKind::Class,
            SymbolKind::Enum,
            SymbolKind::EnumMember,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class, SymbolKind::Struct]),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Struct],
    ),
];

const NIM_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("int", PrimKind::Int),
    ("int8", PrimKind::Int),
    ("int16", PrimKind::Int),
    ("int32", PrimKind::Int),
    ("int64", PrimKind::Int),
    ("uint", PrimKind::Int),
    ("float", PrimKind::Float),
    ("float32", PrimKind::Float),
    ("float64", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("string", PrimKind::Str),
    ("char", PrimKind::Str),
];

pub const NIM_PROFILE: LanguageProfile = LanguageProfile {
    id: "nim",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Future"],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: NIM_PRIMITIVES,
    kind_compatible_table: NIM_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: Some(super::predicates::is_nim_system_magic),
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
    external_by_import: Some(crate::type_checker::profile::language_profile::ExternalByImport),
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    module_scope: crate::type_checker::profile::language_profile::ModuleScope::Off,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    namespace_imports_are_wildcards: false,
    delegate_wrappers: &[],
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::FileStemOrDir,
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
    file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
    alias_module_qname: false,
    module_prefix_rewrites:
        crate::type_checker::profile::language_profile::ModulePrefixRewrites::Off,
    workspace_packages: false,
    reexport_barrel_stems: &["index"],
    self_package_root: None,
    wildcard_workspace_scope: false,
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
    doc_comment_kinds: &["##"],
    visibility_keywords: &[],
    function_prototype_types: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
