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
    namespace_decline: None,
    // `sys`/`az` are namespace aliases over the bicep-runtime ambient symbols
    // (both members land under `bicep.builtins`/`bicep.decorators`), not qname
    // path segments. Strip the alias so `sys.concat`/`az.resourceId` resolve
    // against the bare ambient symbol.
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &["sys", "az"],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    package_by_directory: false,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
    file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["//"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
