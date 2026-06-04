use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const ODIN_KIND_TABLE: KindTable = &[
    (EdgeKind::Calls, &[SymbolKind::Function]),
    (EdgeKind::TypeRef, &[SymbolKind::Struct, SymbolKind::Enum, SymbolKind::TypeAlias]),
];

const ODIN_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("int", PrimKind::Int),
    ("uint", PrimKind::Int),
    ("f32", PrimKind::Float),
    ("f64", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("string", PrimKind::Str),
    ("rune", PrimKind::Str),
];

pub const ODIN_PROFILE: LanguageProfile = LanguageProfile {
    id: "odin",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: false,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: ODIN_PRIMITIVES,
    kind_compatible_table: ODIN_KIND_TABLE,
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
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["//"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
