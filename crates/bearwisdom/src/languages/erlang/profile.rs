// =============================================================================
// languages/erlang/profile.rs — LanguageProfile for Erlang.
//
// Registered in shadow mode. Erlang dispatch is message-passing /
// behaviour-driven; the chain walker's receiver-dispatch model doesn't
// fit. Engine takeover for Erlang waits on a hooks-driven dispatch path.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const ERLANG_KIND_TABLE: KindTable = &[
    (EdgeKind::Calls, &[SymbolKind::Function]),
    (EdgeKind::TypeRef, &[SymbolKind::Module, SymbolKind::TypeAlias]),
];

const ERLANG_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("integer", PrimKind::Int),
    ("float", PrimKind::Float),
    ("atom", PrimKind::Symbol),
    ("binary", PrimKind::Str),
    ("boolean", PrimKind::Bool),
];

pub const ERLANG_PROFILE: LanguageProfile = LanguageProfile {
    id: "erlang",
    qname_separator: ":",
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
    primitive_mapping: ERLANG_PRIMITIVES,
    kind_compatible_table: ERLANG_KIND_TABLE,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["%%"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
