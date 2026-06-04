// =============================================================================
// languages/ocaml/profile.rs — LanguageProfile for OCaml.
//
// Registered in shadow mode. OCaml's module-level dispatch and structural
// typing on records / variants need hook coverage before engine takeover.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const OCAML_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Module]),
    (
        EdgeKind::Implements,
        &[SymbolKind::Module, SymbolKind::Interface],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::Struct,
            SymbolKind::TypeAlias,
            SymbolKind::Module,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Constructor],
    ),
];

const OCAML_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("int", PrimKind::Int),
    ("float", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("string", PrimKind::Str),
    ("char", PrimKind::Str),
    ("unit", PrimKind::Unit),
];

pub const OCAML_PROFILE: LanguageProfile = LanguageProfile {
    id: "ocaml",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Structural,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Lwt.t", "Async.Deferred.t"],
    iterator_method: None,
    primitive_mapping: OCAML_PRIMITIVES,
    kind_compatible_table: OCAML_KIND_TABLE,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["(**"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
