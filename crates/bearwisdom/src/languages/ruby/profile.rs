// =============================================================================
// languages/ruby/profile.rs — LanguageProfile for Ruby.
//
// Engine-side type-system data for Ruby. Registered in shadow mode —
// `engine_primary` stays `false` until ±0.1pp recapture validation lands.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ConstructorPattern, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const RUBY_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Method,
            SymbolKind::Function,
            SymbolKind::Constructor,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class]),
    (
        EdgeKind::Implements,
        &[SymbolKind::Module, SymbolKind::Interface],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Module,
            SymbolKind::Interface,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Module],
    ),
];

const RUBY_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("String", PrimKind::Str),
    ("Symbol", PrimKind::Symbol),
    ("Integer", PrimKind::Int),
    ("Fixnum", PrimKind::Int),
    ("Float", PrimKind::Float),
    ("TrueClass", PrimKind::Bool),
    ("FalseClass", PrimKind::Bool),
    ("NilClass", PrimKind::Unit),
];

/// Ruby profile.
pub const RUBY_PROFILE: LanguageProfile = LanguageProfile {
    id: "ruby",
    qname_separator: "::",
    self_keywords: &["self"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: Some("each"),
    primitive_mapping: RUBY_PRIMITIVES,
    kind_compatible_table: RUBY_KIND_TABLE,
    engine_primary: false,
    constructor_patterns: &[ConstructorPattern::ClassDotNew],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["#"],
    visibility_keywords: &[
        ("public", Visibility::Public),
        ("private", Visibility::Private),
        ("protected", Visibility::Protected),
    ],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
