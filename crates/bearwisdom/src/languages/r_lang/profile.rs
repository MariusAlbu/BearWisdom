// LanguageProfile for R in shadow mode. R S4 dispatch is multi-arg;
// engine takeover waits on DispatchAxis::MultiArg hooks.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ConstructorPattern, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const R_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[SymbolKind::Function, SymbolKind::Method],
    ),
    (EdgeKind::TypeRef, &[SymbolKind::Class]),
    (EdgeKind::Instantiates, &[SymbolKind::Class]),
];

const R_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("integer", PrimKind::Int),
    ("numeric", PrimKind::Float),
    ("double", PrimKind::Float),
    ("logical", PrimKind::Bool),
    ("character", PrimKind::Str),
    ("NULL", PrimKind::Unit),
];

pub const R_PROFILE: LanguageProfile = LanguageProfile {
    id: "r",
    qname_separator: "::",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::MultiArg,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: R_PRIMITIVES,
    kind_compatible_table: R_KIND_TABLE,
    constructor_patterns: &[
        ConstructorPattern::R6DollarNew,
        ConstructorPattern::S4New,
    ],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["#'"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
