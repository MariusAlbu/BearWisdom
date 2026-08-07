// LanguageProfile for Zig in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

// The extractor emits Struct/Enum for type declarations (never Class) and only
// `Calls` / `TypeRef` ref edges. The Calls row adds Variable for the
// `const assert = std.debug.assert` idiom, which binds a callable to a Variable
// symbol.
const ZIG_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Variable,
        ],
    ),
    (
        EdgeKind::TypeRef,
        &[SymbolKind::Struct, SymbolKind::Enum, SymbolKind::TypeAlias],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Struct, SymbolKind::Enum],
    ),
];

const ZIG_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("i8", PrimKind::Int),
    ("i16", PrimKind::Int),
    ("i32", PrimKind::Int),
    ("i64", PrimKind::Int),
    ("u8", PrimKind::Int),
    ("u16", PrimKind::Int),
    ("u32", PrimKind::Int),
    ("u64", PrimKind::Int),
    ("usize", PrimKind::Int),
    ("isize", PrimKind::Int),
    ("f32", PrimKind::Float),
    ("f64", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("void", PrimKind::Unit),
    ("noreturn", PrimKind::Never),
];

pub const ZIG_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    id: "zig",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Structural,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: ZIG_PRIMITIVES,
    kind_compatible_table: ZIG_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: Some(super::predicates::is_zig_builtin),
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
    namespace_imports_are_wildcards: false,
    delegate_wrappers: &[],
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
    doc_comment_kinds: &["//!", "///"],
    visibility_keywords: &[],
    function_prototype_types: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
