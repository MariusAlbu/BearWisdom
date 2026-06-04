// =============================================================================
// languages/kotlin/profile.rs — LanguageProfile for Kotlin.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable, LanguageProfile,
    SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const KOTLIN_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
            SymbolKind::Property,
        ],
    ),
    (
        EdgeKind::Inherits,
        &[SymbolKind::Class, SymbolKind::Interface],
    ),
    (EdgeKind::Implements, &[SymbolKind::Interface]),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Constructor],
    ),
];

const KOTLIN_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("Int", PrimKind::Int),
    ("Long", PrimKind::Int),
    ("Short", PrimKind::Int),
    ("Byte", PrimKind::Int),
    ("UInt", PrimKind::Int),
    ("ULong", PrimKind::Int),
    ("UShort", PrimKind::Int),
    ("UByte", PrimKind::Int),
    ("Float", PrimKind::Float),
    ("Double", PrimKind::Float),
    ("Boolean", PrimKind::Bool),
    ("String", PrimKind::Str),
    ("Char", PrimKind::Str),
    ("Unit", PrimKind::Unit),
    ("Nothing", PrimKind::Never),
    ("Any", PrimKind::Unknown),
];

pub const KOTLIN_PROFILE: LanguageProfile = LanguageProfile {
    id: "kotlin",
    qname_separator: ".",
    self_keywords: &["this", "super"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Deferred", "Flow"],
    iterator_method: Some("iterator"),
    primitive_mapping: KOTLIN_PRIMITIVES,
    kind_compatible_table: KOTLIN_KIND_TABLE,
    // JVM package visibility: members are keyed under package-qualified qnames,
    // so a bare mid-chain receiver (`Repository`, or a same-package return type)
    // qualifies via its package then explicit imports before member lookup.
    chain_qualification: ChainQualification::SamePackageAndImports,
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
    constructor_patterns: &[ConstructorPattern::CallableClass],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AtPrefix),
    doc_comment_kinds: &["/**"],
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
