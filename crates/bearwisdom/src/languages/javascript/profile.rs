// LanguageProfile for JavaScript (when claimed by the dedicated JS
// plugin rather than the TS plugin). Mirrors the TS shape but without
// type-system specifics — JS doesn't carry type annotations.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const JS_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Variable,
            SymbolKind::Property,
            SymbolKind::Class,
            SymbolKind::Constructor,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class]),
    (EdgeKind::TypeRef, &[SymbolKind::Class]),
    (EdgeKind::Instantiates, &[SymbolKind::Class]),
];

const JS_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("string", PrimKind::Str),
    ("number", PrimKind::Float),
    ("bigint", PrimKind::Int),
    ("boolean", PrimKind::Bool),
    ("symbol", PrimKind::Symbol),
    ("undefined", PrimKind::Unit),
    ("null", PrimKind::Unit),
];

pub const JAVASCRIPT_PROFILE: LanguageProfile = LanguageProfile {
    id: "javascript",
    qname_separator: ".",
    self_keywords: &["this"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Promise"],
    iterator_method: None,
    primitive_mapping: JS_PRIMITIVES,
    kind_compatible_table: JS_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    constructor_patterns: &[ConstructorPattern::New, ConstructorPattern::CallableClass],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["/**"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
