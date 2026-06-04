// LanguageProfile for Nim in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const NIM_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[SymbolKind::Function, SymbolKind::Method, SymbolKind::Constructor],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class, SymbolKind::Struct]),
    (
        EdgeKind::TypeRef,
        &[SymbolKind::Class, SymbolKind::Struct, SymbolKind::Enum, SymbolKind::TypeAlias],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class, SymbolKind::Struct]),
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
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Future"],
    iterator_method: None,
    primitive_mapping: NIM_PRIMITIVES,
    kind_compatible_table: NIM_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["##"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
