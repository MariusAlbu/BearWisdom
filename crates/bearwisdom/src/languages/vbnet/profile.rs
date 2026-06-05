// LanguageProfile for VB.NET in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const VBNET_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
            SymbolKind::Property,
            SymbolKind::Delegate,
            SymbolKind::Event,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class, SymbolKind::Struct]),
    (EdgeKind::Implements, &[SymbolKind::Interface]),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::EnumMember,
            SymbolKind::Struct,
            SymbolKind::TypeAlias,
            SymbolKind::Delegate,
        ],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class, SymbolKind::Struct]),
];

const VBNET_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("Integer", PrimKind::Int),
    ("Long", PrimKind::Int),
    ("Short", PrimKind::Int),
    ("Byte", PrimKind::Int),
    ("Single", PrimKind::Float),
    ("Double", PrimKind::Float),
    ("Decimal", PrimKind::Float),
    ("Boolean", PrimKind::Bool),
    ("String", PrimKind::Str),
    ("Char", PrimKind::Str),
];

pub const VBNET_PROFILE: LanguageProfile = LanguageProfile {
    id: "vbnet",
    qname_separator: ".",
    self_keywords: &["Me", "MyClass", "MyBase"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Task"],
    iterator_method: None,
    primitive_mapping: VBNET_PRIMITIVES,
    kind_compatible_table: VBNET_KIND_TABLE,
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
    alias_module_qname: false,
    module_prefix_rewrites: crate::type_checker::profile::language_profile::ModulePrefixRewrites::Off,
    workspace_packages: false,
    overload_pick_all: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::Off,
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    namespaceless_global_type_lookup: false,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["'''"],
    visibility_keywords: &[
        ("Public", Visibility::Public),
        ("Private", Visibility::Private),
        ("Protected", Visibility::Protected),
        ("Friend", Visibility::Internal),
    ],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
