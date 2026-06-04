// LanguageProfile for Gleam in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const GLEAM_KIND_TABLE: KindTable = &[
    (EdgeKind::Calls, &[SymbolKind::Function]),
    (
        EdgeKind::TypeRef,
        &[SymbolKind::TypeAlias, SymbolKind::Enum, SymbolKind::Struct, SymbolKind::Module],
    ),
];

const GLEAM_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("Int", PrimKind::Int),
    ("Float", PrimKind::Float),
    ("Bool", PrimKind::Bool),
    ("String", PrimKind::Str),
    ("Nil", PrimKind::Unit),
];

pub const GLEAM_PROFILE: LanguageProfile = LanguageProfile {
    id: "gleam",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: GLEAM_PRIMITIVES,
    kind_compatible_table: GLEAM_KIND_TABLE,
    // A Gleam import (`import gleam/io`) names a module; its members are keyed
    // under the path's last segment (`io.println`). A bare member ref whose
    // qualifier the extractor dropped resolves under `{short}.{target}` — the
    // same shape the engine's package-short-name strategy binds.
    chain_qualification: ChainQualification::PackageShortName,
    builtin_skip: Some(super::hooks::is_gleam_operator),
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["///"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
