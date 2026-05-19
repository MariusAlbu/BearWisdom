// =============================================================================
// languages/java/profile.rs — LanguageProfile for Java.
//
// Phase 6 wave-A migration.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable, LanguageProfile,
    SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const JAVA_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[SymbolKind::Method, SymbolKind::Function, SymbolKind::Constructor],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class]),
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
    (EdgeKind::Instantiates, &[SymbolKind::Class]),
];

const JAVA_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("byte", PrimKind::Int),
    ("short", PrimKind::Int),
    ("int", PrimKind::Int),
    ("long", PrimKind::Int),
    ("float", PrimKind::Float),
    ("double", PrimKind::Float),
    ("char", PrimKind::Char),
    ("boolean", PrimKind::Bool),
    ("void", PrimKind::Unit),
    ("String", PrimKind::Str),
];

pub const JAVA_PROFILE: LanguageProfile = LanguageProfile {
    id: "java",
    qname_separator: ".",
    self_keywords: &["this", "super"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    // The JDK source is the external surface; Maven sources jars too.
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    // Sealed classes (Java 17+) are a form of sum types but engine
    // narrowing isn't wired; conservative off.
    has_sum_types: false,
    // java.util.Optional is a regular class with member methods (`get`,
    // `orElse`); the engine doesn't transparently unwrap it. Setting
    // this true would cause method lookups on Optional<T> to fall
    // through to T's members, missing Optional's own surface.
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &["CompletableFuture", "Future"],
    iterator_method: Some("iterator"),
    primitive_mapping: JAVA_PRIMITIVES,
    kind_compatible_table: JAVA_KIND_TABLE,
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
