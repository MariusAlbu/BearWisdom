// LanguageProfile for Zig in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const ZIG_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[SymbolKind::Function, SymbolKind::Method],
    ),
    (
        EdgeKind::TypeRef,
        &[SymbolKind::Struct, SymbolKind::Enum, SymbolKind::TypeAlias],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Struct, SymbolKind::Enum]),
];

const ZIG_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("i8", PrimKind::Int),
    ("i16", PrimKind::Int),
    ("i32", PrimKind::Int),
    ("i64", PrimKind::Int),
    ("u8", PrimKind::Int),
    ("u16", PrimKind::Int),
    ("u32", PrimKind::Int),
    ("u64", PrimKind::Int),
    ("usize", PrimKind::Int),
    ("isize", PrimKind::Int),
    ("f32", PrimKind::Float),
    ("f64", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("void", PrimKind::Unit),
    ("noreturn", PrimKind::Never),
];

pub const ZIG_PROFILE: LanguageProfile = LanguageProfile {
    id: "zig",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Structural,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: ZIG_PRIMITIVES,
    kind_compatible_table: ZIG_KIND_TABLE,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["//!", "///"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
