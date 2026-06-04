// =============================================================================
// languages/clojure/profile.rs — LanguageProfile for Clojure.
//
// Registered in shadow mode. Clojure multimethod dispatch is multi-arg /
// hierarchy-driven; engine takeover waits on DispatchAxis::MultiArg hooks.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const CLOJURE_KIND_TABLE: KindTable = &[
    (EdgeKind::Calls, &[SymbolKind::Function, SymbolKind::Method]),
    (
        EdgeKind::TypeRef,
        &[SymbolKind::Module, SymbolKind::TypeAlias],
    ),
];

const CLOJURE_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("long", PrimKind::Int),
    ("int", PrimKind::Int),
    ("double", PrimKind::Float),
    ("float", PrimKind::Float),
    ("boolean", PrimKind::Bool),
    ("String", PrimKind::Str),
    ("nil", PrimKind::Unit),
];

pub const CLOJURE_PROFILE: LanguageProfile = LanguageProfile {
    id: "clojure",
    qname_separator: "/",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Structural,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::MultiArg,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: CLOJURE_PRIMITIVES,
    kind_compatible_table: CLOJURE_KIND_TABLE,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &[";;"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
