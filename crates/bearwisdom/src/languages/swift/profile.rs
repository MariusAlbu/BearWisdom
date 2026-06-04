// =============================================================================
// languages/swift/profile.rs — LanguageProfile for Swift.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable, LanguageProfile,
    SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const SWIFT_KIND_TABLE: KindTable = &[
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
        &[SymbolKind::Interface, SymbolKind::Trait],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Struct,
            SymbolKind::Interface,
            SymbolKind::Trait,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Struct, SymbolKind::Enum],
    ),
];

const SWIFT_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("Int", PrimKind::Int),
    ("Int8", PrimKind::Int),
    ("Int16", PrimKind::Int),
    ("Int32", PrimKind::Int),
    ("Int64", PrimKind::Int),
    ("UInt", PrimKind::Int),
    ("UInt8", PrimKind::Int),
    ("UInt16", PrimKind::Int),
    ("UInt32", PrimKind::Int),
    ("UInt64", PrimKind::Int),
    ("Float", PrimKind::Float),
    ("Double", PrimKind::Float),
    ("Bool", PrimKind::Bool),
    ("String", PrimKind::Str),
    ("Character", PrimKind::Str),
    ("Void", PrimKind::Unit),
    ("Never", PrimKind::Never),
    ("Any", PrimKind::Unknown),
    ("AnyObject", PrimKind::Unknown),
];

pub const SWIFT_PROFILE: LanguageProfile = LanguageProfile {
    id: "swift",
    qname_separator: ".",
    self_keywords: &["self", "Self", "super"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Task", "AsyncSequence"],
    iterator_method: Some("makeIterator"),
    primitive_mapping: SWIFT_PRIMITIVES,
    kind_compatible_table: SWIFT_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    ambient_namespace_prefixes: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    constructor_patterns: &[ConstructorPattern::CallableClass],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AtPrefix),
    doc_comment_kinds: &["///"],
    visibility_keywords: &[
        ("public", Visibility::Public),
        ("open", Visibility::Public),
        ("private", Visibility::Private),
        ("fileprivate", Visibility::Private),
        ("internal", Visibility::Internal),
    ],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
