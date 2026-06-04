// =============================================================================
// languages/haskell/profile.rs — LanguageProfile for Haskell.
//
// Registered in shadow mode. Haskell typeclass dispatch is return-type-
// driven; engine takeover waits on a DispatchAxis::ReturnType-aware path.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const HASKELL_KIND_TABLE: KindTable = &[
    (EdgeKind::Calls, &[SymbolKind::Function]),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Trait,
            SymbolKind::Interface,
            SymbolKind::TypeAlias,
            SymbolKind::Enum,
            SymbolKind::Module,
        ],
    ),
    (EdgeKind::Implements, &[SymbolKind::Class, SymbolKind::Trait]),
    (EdgeKind::Inherits, &[SymbolKind::Class, SymbolKind::Trait]),
];

const HASKELL_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("Int", PrimKind::Int),
    ("Integer", PrimKind::Int),
    ("Float", PrimKind::Float),
    ("Double", PrimKind::Float),
    ("Bool", PrimKind::Bool),
    ("Char", PrimKind::Str),
    ("String", PrimKind::Str),
    ("()", PrimKind::Unit),
];

pub const HASKELL_PROFILE: LanguageProfile = LanguageProfile {
    id: "haskell",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::ReturnType,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["IO"],
    iterator_method: None,
    primitive_mapping: HASKELL_PRIMITIVES,
    kind_compatible_table: HASKELL_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["-- |"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
