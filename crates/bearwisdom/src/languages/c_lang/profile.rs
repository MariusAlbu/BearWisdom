// LanguageProfile for C/C++ in shadow mode.

use super::predicates;
use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const C_KIND_TABLE: KindTable = &[
    (EdgeKind::Calls, &[SymbolKind::Function, SymbolKind::Method, SymbolKind::Constructor]),
    (EdgeKind::Inherits, &[SymbolKind::Class, SymbolKind::Struct]),
    (
        EdgeKind::TypeRef,
        &[SymbolKind::Class, SymbolKind::Struct, SymbolKind::Enum, SymbolKind::TypeAlias],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class, SymbolKind::Struct]),
];

const C_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("char", PrimKind::Str),
    ("int", PrimKind::Int),
    ("short", PrimKind::Int),
    ("long", PrimKind::Int),
    ("size_t", PrimKind::Int),
    ("ssize_t", PrimKind::Int),
    ("float", PrimKind::Float),
    ("double", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("void", PrimKind::Unit),
];

pub const C_LANG_PROFILE: LanguageProfile = LanguageProfile {
    id: "c",
    qname_separator: "::",
    self_keywords: &["this"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: false,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: C_PRIMITIVES,
    kind_compatible_table: C_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    // Template parameters (`T`, `U`, `_Range`, `<...>`, `this`, `nullptr`, …)
    // are not project symbols; decline them before the bare-name ladder so they
    // are never bound to a same-named project symbol or seeded as a chain miss.
    builtin_skip: Some(predicates::is_template_param),
    // R-package native C sources: a `.C("LENGTH")` callee names the R C API,
    // not a same-named project symbol. Declined before the ladder only when
    // the file carries the R-package namespace; external classification then
    // brands it `r.c.api`.
    namespace_decline: Some(crate::type_checker::profile::language_profile::NamespaceDecline {
        file_namespace: super::hooks::R_PACKAGE_SENTINEL,
        is_reserved: predicates::is_r_c_api_symbol,
    }),
    ambient_namespace_prefixes: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["//", "/*", "/**"],
    visibility_keywords: &[
        ("public", Visibility::Public),
        ("private", Visibility::Private),
        ("protected", Visibility::Protected),
    ],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
