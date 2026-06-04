// =============================================================================
// languages/dart/profile.rs — LanguageProfile for Dart.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable, LanguageProfile,
    SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const DART_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
        ],
    ),
    (
        EdgeKind::Inherits,
        &[SymbolKind::Class],
    ),
    (
        EdgeKind::Implements,
        &[SymbolKind::Class, SymbolKind::Interface],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
        ],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class]),
];

const DART_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("int", PrimKind::Int),
    ("double", PrimKind::Float),
    ("num", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("String", PrimKind::Str),
    ("void", PrimKind::Unit),
    ("Null", PrimKind::Unit),
    ("Never", PrimKind::Never),
    ("dynamic", PrimKind::Unknown),
    ("Object", PrimKind::Unknown),
];

pub const DART_PROFILE: LanguageProfile = LanguageProfile {
    id: "dart",
    qname_separator: ".",
    self_keywords: &["this", "super"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Future", "Stream"],
    iterator_method: Some("iterator"),
    primitive_mapping: DART_PRIMITIVES,
    kind_compatible_table: DART_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    // Library-prefix bind: a `i0.Value` ref carries the prefix's import URI on
    // `module`. Resolve that URI to its project file via `in_module_from` and
    // bind the bare name there; on a miss, terminate so an external prefix is
    // not hijacked by a same-named local symbol.
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::On(
        crate::type_checker::profile::language_profile::ModuleAnchorBind::NameExactKind,
    ),
    module_anchor_terminal: true,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    package_by_directory: false,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
    file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
    constructor_patterns: &[
        ConstructorPattern::New,
        ConstructorPattern::CallableClass,
    ],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AtPrefix),
    doc_comment_kinds: &["///"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
