// =============================================================================
// languages/csharp/profile.rs — LanguageProfile for C#.
//
// Phase 6 wave-A migration.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable, LanguageProfile,
    SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const CS_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Method,
            SymbolKind::Function,
            SymbolKind::Constructor,
            SymbolKind::Delegate,
            SymbolKind::Property,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class]),
    (EdgeKind::Implements, &[SymbolKind::Interface]),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
            SymbolKind::Delegate,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Struct],
    ),
];

const CS_PRIMITIVES: &[(&str, PrimKind)] = &[
    // Both keyword and BCL-typed names — extractor surfaces both.
    ("string", PrimKind::Str),
    ("String", PrimKind::Str),
    ("byte", PrimKind::Int),
    ("sbyte", PrimKind::Int),
    ("short", PrimKind::Int),
    ("ushort", PrimKind::Int),
    ("int", PrimKind::Int),
    ("uint", PrimKind::Int),
    ("long", PrimKind::Int),
    ("ulong", PrimKind::Int),
    ("Int32", PrimKind::Int),
    ("Int64", PrimKind::Int),
    ("float", PrimKind::Float),
    ("double", PrimKind::Float),
    ("decimal", PrimKind::Float),
    ("char", PrimKind::Char),
    ("bool", PrimKind::Bool),
    ("Boolean", PrimKind::Bool),
    ("void", PrimKind::Unit),
    ("object", PrimKind::Unknown),
    ("dynamic", PrimKind::Unknown),
];

pub const CSHARP_PROFILE: LanguageProfile = LanguageProfile {
    id: "csharp",
    qname_separator: ".",
    self_keywords: &["this", "base"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    // NuGet metadata + dotnet-stdlib externals.
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: false,
    // Nullable<T> wraps value types in C#; engine doesn't transparently
    // unwrap it. Reference-type Nullable annotations (`string?`) are
    // metadata only.
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &["Task", "ValueTask"],
    container_accessors: &[],
    iterator_method: Some("GetEnumerator"),
    primitive_mapping: CS_PRIMITIVES,
    kind_compatible_table: CS_KIND_TABLE,
    // Same-namespace + using-directive qualification of a bare receiver type:
    // the structured walker's `qualify_current_ty` reproduces the .NET
    // qualification the C# resolver did by hand.
    chain_qualification: ChainQualification::SamePackageAndImports,
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
    alias_module_qname: false,
    module_prefix_rewrites: crate::type_checker::profile::language_profile::ModulePrefixRewrites::Off,
    workspace_packages: false,
    overload_pick_all: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::Off,
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    namespaceless_global_type_lookup: false,
    constructor_patterns: &[ConstructorPattern::New],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AttrBracket),
    doc_comment_kinds: &["///"],
    visibility_keywords: &[
        ("public", Visibility::Public),
        ("private", Visibility::Private),
        ("protected", Visibility::Protected),
        ("internal", Visibility::Internal),
    ],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
