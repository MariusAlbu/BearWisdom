// =============================================================================
// languages/lua/profile.rs — LanguageProfile for Lua.
//
// Lua's metatable-based OO means full member resolution needs hooks for
// `setmetatable(t, M)` and the `__index` chain.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
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
    (
        EdgeKind::TypeRef,
        &[SymbolKind::Class, SymbolKind::Module],
    ),
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
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: LUA_PRIMITIVES,
    kind_compatible_table: LUA_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    constructor_patterns: &[ConstructorPattern::LuaColonNew],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["---"],
    visibility_keywords: &[("local", Visibility::Private)],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
