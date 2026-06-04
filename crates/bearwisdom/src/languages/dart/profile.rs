// =============================================================================
// languages/dart/profile.rs — LanguageProfile for Dart.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable, LanguageProfile,
    SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const DART_KIND_TABLE: KindTable = &[
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
        &[SymbolKind::Class],
    ),
    (
        EdgeKind::Implements,
        &[SymbolKind::Class, SymbolKind::Interface],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
        ],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class]),
];

const DART_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("int", PrimKind::Int),
    ("double", PrimKind::Float),
    ("num", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("String", PrimKind::Str),
    ("void", PrimKind::Unit),
    ("Null", PrimKind::Unit),
    ("Never", PrimKind::Never),
    ("dynamic", PrimKind::Unknown),
    ("Object", PrimKind::Unknown),
];

pub const DART_PROFILE: LanguageProfile = LanguageProfile {
    id: "dart",
    qname_separator: ".",
    self_keywords: &["this", "super"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Future", "Stream"],
    iterator_method: Some("iterator"),
    primitive_mapping: DART_PRIMITIVES,
    kind_compatible_table: DART_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    constructor_patterns: &[
        ConstructorPattern::New,
        ConstructorPattern::CallableClass,
    ],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AtPrefix),
    doc_comment_kinds: &["///"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
