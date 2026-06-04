// LanguageProfile for Ada in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const ADA_KIND_TABLE: KindTable = &[
    (EdgeKind::Calls, &[SymbolKind::Function, SymbolKind::Method, SymbolKind::Constructor]),
    (EdgeKind::Inherits, &[SymbolKind::Class, SymbolKind::Struct]),
    (
        EdgeKind::TypeRef,
        &[SymbolKind::Class, SymbolKind::Struct, SymbolKind::Enum, SymbolKind::TypeAlias, SymbolKind::Module],
    ),
];

const ADA_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("Integer", PrimKind::Int),
    ("Natural", PrimKind::Int),
    ("Positive", PrimKind::Int),
    ("Float", PrimKind::Float),
    ("Long_Float", PrimKind::Float),
    ("Boolean", PrimKind::Bool),
    ("Character", PrimKind::Str),
    ("String", PrimKind::Str),
];

pub const ADA_PROFILE: LanguageProfile = LanguageProfile {
    id: "ada",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: ADA_PRIMITIVES,
    kind_compatible_table: ADA_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
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
    doc_comment_kinds: &["--"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
