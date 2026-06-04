// =============================================================================
// languages/scala/profile.rs — LanguageProfile for Scala.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const SCALA_KIND_TABLE: KindTable = &[
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
        &[SymbolKind::Class, SymbolKind::Trait],
    ),
    (
        EdgeKind::Implements,
        &[SymbolKind::Trait, SymbolKind::Interface],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Trait,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
            SymbolKind::Module,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Module],
    ),
];

const SCALA_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("Int", PrimKind::Int),
    ("Long", PrimKind::Int),
    ("Short", PrimKind::Int),
    ("Byte", PrimKind::Int),
    ("Float", PrimKind::Float),
    ("Double", PrimKind::Float),
    ("Boolean", PrimKind::Bool),
    ("String", PrimKind::Str),
    ("Char", PrimKind::Str),
    ("Unit", PrimKind::Unit),
    ("Nothing", PrimKind::Never),
    ("Any", PrimKind::Unknown),
    ("AnyRef", PrimKind::Unknown),
    ("AnyVal", PrimKind::Unknown),
];

pub const SCALA_PROFILE: LanguageProfile = LanguageProfile {
    id: "scala",
    qname_separator: ".",
    self_keywords: &["this", "super"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Future", "IO", "Task"],
    iterator_method: Some("iterator"),
    primitive_mapping: SCALA_PRIMITIVES,
    kind_compatible_table: SCALA_KIND_TABLE,
    // JVM package visibility: members are keyed under package-qualified qnames,
    // so a bare mid-chain receiver (a same-package type or one named by an
    // explicit import) qualifies before member lookup.
    chain_qualification: ChainQualification::SamePackageAndImports,
    builtin_skip: None,
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    constructor_patterns: &[
        crate::type_checker::profile::language_profile::ConstructorPattern::New,
        crate::type_checker::profile::language_profile::ConstructorPattern::CallableClass,
    ],
    class_builder_specs: &[],
    decorator_syntax: Some(
        crate::type_checker::profile::language_profile::DecoratorSyntax::AtPrefix,
    ),
    doc_comment_kinds: &["/**"],
    visibility_keywords: &[
        ("private", Visibility::Private),
        ("protected", Visibility::Protected),
    ],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
