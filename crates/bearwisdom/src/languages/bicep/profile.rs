// Minimal LanguageProfile for Bicep in shadow mode.

use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

// Bicep user-defined functions and types; ARM resource API methods land as
// methods. `variable`/`function` are valid TypeRef targets (`param x type` and
// user-defined functions referenced as types).
const BICEP_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[SymbolKind::Method, SymbolKind::Function, SymbolKind::Constructor],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
            SymbolKind::Variable,
            SymbolKind::Function,
        ],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class, SymbolKind::Function]),
];

pub const BICEP_PROFILE: LanguageProfile = LanguageProfile {
    id: "bicep",
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
    primitive_mapping: &[],
    kind_compatible_table: BICEP_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: Some(super::hooks::is_azure_resource_type),
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
