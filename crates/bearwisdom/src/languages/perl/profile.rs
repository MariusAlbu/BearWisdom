// LanguageProfile for Perl in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const PERL_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[SymbolKind::Function, SymbolKind::Method, SymbolKind::Constructor],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class, SymbolKind::Module]),
    (
        EdgeKind::TypeRef,
        &[SymbolKind::Class, SymbolKind::Module],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class, SymbolKind::Module]),
];

const PERL_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("scalar", PrimKind::Unknown),
    ("array", PrimKind::Unknown),
    ("hash", PrimKind::Unknown),
];

pub const PERL_PROFILE: LanguageProfile = LanguageProfile {
    id: "perl",
    qname_separator: "::",
    self_keywords: &["$self"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: PERL_PRIMITIVES,
    kind_compatible_table: PERL_KIND_TABLE,
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
    package_by_directory: false,
    constructor_patterns: &[ConstructorPattern::ArrowNew],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["#", "=pod"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
