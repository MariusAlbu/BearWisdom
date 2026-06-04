// LanguageProfile for MATLAB in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const MATLAB_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[SymbolKind::Function, SymbolKind::Method],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class]),
    (
        EdgeKind::TypeRef,
        &[SymbolKind::Class, SymbolKind::Module],
    ),
];

const MATLAB_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("double", PrimKind::Float),
    ("single", PrimKind::Float),
    ("int8", PrimKind::Int),
    ("int16", PrimKind::Int),
    ("int32", PrimKind::Int),
    ("int64", PrimKind::Int),
    ("logical", PrimKind::Bool),
    ("char", PrimKind::Str),
    ("string", PrimKind::Str),
];

pub const MATLAB_PROFILE: LanguageProfile = LanguageProfile {
    id: "matlab",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: false,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: MATLAB_PRIMITIVES,
    kind_compatible_table: MATLAB_KIND_TABLE,
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
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["%"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
