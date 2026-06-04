// =============================================================================
// languages/typescript/profile.rs — LanguageProfile for TypeScript / TSX / JS.
//
// Single source of truth for TS-specific type-system behaviour the engine
// consumes. Filled per doc 3 (LanguageProfile spec); reused for JavaScript,
// JSX, TSX, Vue's <script> and Svelte's <script> via the same plugin.
//
// Phase 5 of the engine pivot.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable, LanguageProfile,
    SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

/// Kind-compatibility table for TypeScript.
///
/// Encodes which symbol kinds are valid resolution targets per edge kind:
///   - Calls: function / method / variable (callable variables) / property
///     (when typed `() => ...`) / class (callable class `Foo()` works in TS
///     after `new`-less constructors via `Object.create` etc., common in
///     practice).
///   - Inherits: class / interface (since TS allows `class X extends Y`
///     where Y can be a class or interface in declaration-merging).
///   - Implements: interface / type_alias (TS allows
///     `class X implements TypeAlias` when the alias resolves to an object).
///   - TypeRef: class / interface / enum / type_alias / struct (parser-
///     emitted for tuples typed as structs).
///   - Instantiates: class / interface (interfaces are constructed in TS
///     when augmented with a callable signature).
const TS_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Variable,
            SymbolKind::Property,
            SymbolKind::Class,
            SymbolKind::Constructor,
        ],
    ),
    (
        EdgeKind::Inherits,
        &[SymbolKind::Class, SymbolKind::Interface],
    ),
    (
        EdgeKind::Implements,
        &[SymbolKind::Interface, SymbolKind::TypeAlias],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
            SymbolKind::Struct,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Interface],
    ),
];

/// Primitive name → engine PrimKind for TypeScript's surface types.
const TS_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("string", PrimKind::Str),
    ("number", PrimKind::Float),
    ("bigint", PrimKind::Int),
    ("boolean", PrimKind::Bool),
    ("symbol", PrimKind::Symbol),
    ("void", PrimKind::Unit),
    ("never", PrimKind::Never),
    ("undefined", PrimKind::Unit),
    ("null", PrimKind::Unit),
    ("unknown", PrimKind::Unknown),
    ("any", PrimKind::Unknown),
];

/// TypeScript profile. Shared across TS / TSX / JS / Vue-script /
/// Svelte-script. JavaScript carries the same dynamic-type semantics on the
/// engine side (declared annotations either come from JSDoc or are absent;
/// inference + member lookup fall back to dynamic property resolution which
/// the engine handles uniformly with TS).
pub const TYPESCRIPT_PROFILE: LanguageProfile = LanguageProfile {
    id: "typescript",
    qname_separator: ".",
    // `this` is the only receiver keyword TS surfaces at the chain-walker
    // root; `super` is handled by the resolver via parent-class lookup
    // (no SelfRef SegmentKind today carries `super`).
    self_keywords: &["this"],
    // Both: TS interfaces are structural ("any object with these members
    // satisfies"), classes are nominal (must `extends`). Engine treats
    // both as supertype edges so member lookup is uniform.
    supertype_discovery: SupertypeDiscovery::Both,
    // Externals: `@types/*` declaration files contribute methods that
    // appear on user types via declaration merging — `Array.prototype.foo`
    // declared in user code participates in resolution alongside lib.dom.
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    // TS preserves literal types in strict-mode `const` declarations but
    // widens elsewhere. The engine's literal-narrowing path is opt-in;
    // leaving it off avoids over-narrowing untyped JS callsites.
    literal_narrowing: false,
    // `await someAsync()` unwraps Promise<T> -> T. Engine handles via
    // AsyncWrapper / unwrap_await + this axis declaring the wrapper class.
    async_wrappers: &["Promise"],
    // TS iteration protocols (Iterable<T>, IterableIterator<T>) are
    // expressed via the `[Symbol.iterator]` method; the engine doesn't
    // unwrap on a single method name — let the chain walker peel via
    // Type::Apply<Array,[T]> args[0] explicitly when needed.
    iterator_method: None,
    primitive_mapping: TS_PRIMITIVES,
    kind_compatible_table: TS_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    // Validated under engine-primary mode at Phase 5 § stage 4 — rate
    // parity ±0.0pp across ts-rallly, vue-vben-admin, ts-immich;
    // ±0.05pp on ts-nextjs. Engine takes the chain slot for TS.
    // TS supports `new Foo()` (NewExpression) and `Foo()` (CallExpression)
    // both as construction; the extractor emits both as Construction
    // segments. Engine accepts both.
    builtin_skip: None,
    namespace_decline: None,
    ambient_namespace_prefixes: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    constructor_patterns: &[
        ConstructorPattern::New,
        ConstructorPattern::CallableClass,
    ],
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
