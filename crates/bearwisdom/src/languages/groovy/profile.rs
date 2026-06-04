// LanguageProfile for Groovy in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable,
    LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const GROOVY_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[SymbolKind::Function, SymbolKind::Method, SymbolKind::Constructor],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class, SymbolKind::Interface]),
    (EdgeKind::Implements, &[SymbolKind::Interface]),
    (
        EdgeKind::TypeRef,
        &[SymbolKind::Class, SymbolKind::Interface, SymbolKind::Enum],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class]),
];

const GROOVY_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("int", PrimKind::Int),
    ("long", PrimKind::Int),
    ("short", PrimKind::Int),
    ("byte", PrimKind::Int),
    ("float", PrimKind::Float),
    ("double", PrimKind::Float),
    ("boolean", PrimKind::Bool),
    ("String", PrimKind::Str),
    ("char", PrimKind::Str),
    ("void", PrimKind::Unit),
];

pub const GROOVY_PROFILE: LanguageProfile = LanguageProfile {
    id: "groovy",
    qname_separator: ".",
    self_keywords: &["this", "super"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: Some("iterator"),
    primitive_mapping: GROOVY_PRIMITIVES,
    kind_compatible_table: GROOVY_KIND_TABLE,
    // Groovy shares Java's package-qualified member keying; same-package +
    // explicit-import qualification of a bare mid-chain receiver.
    chain_qualification: ChainQualification::SamePackageAndImports,
    builtin_skip: None,
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    constructor_patterns: &[ConstructorPattern::New],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AtPrefix),
    doc_comment_kinds: &["/**"],
    visibility_keywords: &[
        ("public", Visibility::Public),
        ("private", Visibility::Private),
        ("protected", Visibility::Protected),
    ],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
