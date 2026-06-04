// =============================================================================
// languages/elixir/profile.rs — LanguageProfile for Elixir.
//
// Registered in shadow mode. Elixir's protocol dispatch and pipe-based
// chains need hook coverage before engine takeover.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const ELIXIR_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Method,
            SymbolKind::Function,
            SymbolKind::Constructor,
            SymbolKind::Test,
            SymbolKind::Property,
            SymbolKind::Module,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class, SymbolKind::Module]),
    (
        EdgeKind::Implements,
        &[SymbolKind::Class, SymbolKind::Module, SymbolKind::Interface],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Module,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
            SymbolKind::Namespace,
        ],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class, SymbolKind::Module]),
];

const ELIXIR_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("integer", PrimKind::Int),
    ("float", PrimKind::Float),
    ("atom", PrimKind::Symbol),
    ("binary", PrimKind::Str),
    ("boolean", PrimKind::Bool),
    ("nil", PrimKind::Unit),
];

pub const ELIXIR_PROFILE: LanguageProfile = LanguageProfile {
    id: "elixir",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: true,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: ELIXIR_PRIMITIVES,
    kind_compatible_table: ELIXIR_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["@doc"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
