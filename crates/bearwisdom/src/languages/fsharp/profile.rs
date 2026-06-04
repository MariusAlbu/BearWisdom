// LanguageProfile for F# in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DecoratorSyntax, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const FSHARP_KIND_TABLE: KindTable = &[
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
            SymbolKind::TypeAlias,
            SymbolKind::Struct,
            SymbolKind::Module,
        ],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class, SymbolKind::Struct]),
];

const FSHARP_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("int", PrimKind::Int),
    ("float", PrimKind::Float),
    ("double", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("string", PrimKind::Str),
    ("char", PrimKind::Str),
    ("unit", PrimKind::Unit),
];

pub const FSHARP_PROFILE: LanguageProfile = LanguageProfile {
    id: "fsharp",
    qname_separator: ".",
    self_keywords: &["this"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Async", "Task"],
    iterator_method: None,
    primitive_mapping: FSHARP_PRIMITIVES,
    kind_compatible_table: FSHARP_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AttrBracket),
    doc_comment_kinds: &["///"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
