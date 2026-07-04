// =============================================================================
// languages/lua/profile.rs — LanguageProfile for Lua.
//
// Lua's metatable-based OO means full member resolution needs hooks for
// `setmetatable(t, M)` and the `__index` chain.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DispatchAxis, KindTable, LanguageProfile,
    SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const LUA_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Variable,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class]),
    (EdgeKind::TypeRef, &[SymbolKind::Class, SymbolKind::Module]),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Module],
    ),
];

const LUA_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("number", PrimKind::Float),
    ("integer", PrimKind::Int),
    ("string", PrimKind::Str),
    ("boolean", PrimKind::Bool),
    ("nil", PrimKind::Unit),
    ("table", PrimKind::Unknown),
    ("function", PrimKind::Unknown),
    ("userdata", PrimKind::Unknown),
];

pub const LUA_PROFILE: LanguageProfile = LanguageProfile {
    id: "lua",
    qname_separator: ".",
    self_keywords: &["self"],
    supertype_discovery: SupertypeDiscovery::Structural,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: LUA_PRIMITIVES,
    kind_compatible_table: LUA_KIND_TABLE,
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
    reexport_barrel_stems: &["index"],
    self_package_root: None,
    wildcard_workspace_scope: false,
    overload_pick_all: false,
    argument_dependent_lookup: false,
    associated_type_projection: false,
    blanket_impl_resolution: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::On {
        instantiate_accepts_variable: false,
    },
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    namespaceless_global_type_lookup:
        crate::type_checker::profile::language_profile::NamespaceScope::Off,
    explicit_member_import: false,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[ConstructorPattern::LuaColonNew],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["---"],
    visibility_keywords: &[("local", Visibility::Private)],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
