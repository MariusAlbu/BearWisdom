// LanguageProfile for Pascal in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const PASCAL_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[SymbolKind::Function, SymbolKind::Method, SymbolKind::Constructor],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class]),
    (
        EdgeKind::TypeRef,
        &[SymbolKind::Class, SymbolKind::Struct, SymbolKind::Enum, SymbolKind::TypeAlias],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class, SymbolKind::Struct]),
];

const PASCAL_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("Integer", PrimKind::Int),
    ("Cardinal", PrimKind::Int),
    ("Word", PrimKind::Int),
    ("Byte", PrimKind::Int),
    ("Real", PrimKind::Float),
    ("Double", PrimKind::Float),
    ("Boolean", PrimKind::Bool),
    ("String", PrimKind::Str),
    ("Char", PrimKind::Str),
];

pub const PASCAL_PROFILE: LanguageProfile = LanguageProfile {
    id: "pascal",
    qname_separator: ".",
    self_keywords: &["Self"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: PASCAL_PRIMITIVES,
    kind_compatible_table: PASCAL_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["{", "(*"],
    visibility_keywords: &[
        ("public", Visibility::Public),
        ("private", Visibility::Private),
        ("protected", Visibility::Protected),
        ("strict private", Visibility::Private),
    ],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
