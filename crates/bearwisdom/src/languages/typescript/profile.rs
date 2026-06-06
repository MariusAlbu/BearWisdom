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
    AccessorSlot, ChainQualification, ConstructorPattern, ContainerShape, DecoratorSyntax,
    DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
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
            SymbolKind::Test,
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
    // TypeRef carries the TS import-binding ref (the extractor emits every
    // `import { X } from '...'` as a TypeRef regardless of X's actual kind),
    // so a function / variable / namespace import must bind here alongside the
    // type kinds.
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
            SymbolKind::Struct,
            SymbolKind::Function,
            SymbolKind::Variable,
            SymbolKind::Namespace,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Interface, SymbolKind::Function],
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
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
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
    // Built-in container accessors that type through the element/value. The
    // yield is projected from the receiver `Apply` args structurally — Array's
    // element-returning methods yield `args[0]`, `Map.get` yields `args[1]`.
    container_accessors: &[
        ("pop", ContainerShape::Sequence, AccessorSlot::Element),
        ("shift", ContainerShape::Sequence, AccessorSlot::Element),
        ("at", ContainerShape::Sequence, AccessorSlot::Element),
        ("find", ContainerShape::Sequence, AccessorSlot::Element),
        ("get", ContainerShape::Map, AccessorSlot::Value),
    ],
    // TS iteration protocols (Iterable<T>, IterableIterator<T>) are
    // expressed via the `[Symbol.iterator]` method; the engine doesn't
    // unwrap on a single method name — let the chain walker peel via
    // Type::Apply<Array,[T]> args[0] explicitly when needed.
    single_inner_wrappers: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: TS_PRIMITIVES,
    kind_compatible_table: TS_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    // === Module-anchored resolution ===
    // A `module` set by the extractor (import source) or post-pass (a call ref's
    // owning module) anchors the bind. A relative `./x` / `../y` specifier binds
    // by exact-name-and-kind in the resolved file (`in_module_from`); every bare
    // specifier (`react`, `@scope/pkg`) routes to the directory/qname rule, which
    // for TS means the qname-prefix rewrites below.
    // TS supports `new Foo()` (NewExpression) and `Foo()` (CallExpression)
    // both as construction; the extractor emits both as Construction
    // segments. The engine accepts both.
    builtin_skip: None,
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    import_resolution: None,
    // Harvest the extractor's `TypeRef`-with-module import refs and the
    // post-pass call refs that carry a `module` into the file's import table.
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::FromModuleField,
    // Relative (`./x`) modules bind by exact name + kind in the resolved file;
    // bare specifiers route to ByNameUnderModuleDir (the qname-rewrite path).
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::On(
        crate::type_checker::profile::language_profile::ModuleAnchorBind::NameExactKind,
    ),
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::DotSlashPrefix,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    package_by_directory: false,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
    file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
    // DefinitelyTyped (`react` → `@types/react`) + deep-import peel
    // (`rxjs/operators` → `rxjs`); a bare specifier never directory-matches a
    // same-named project file.
    alias_module_qname: false,
    module_prefix_rewrites: crate::type_checker::profile::language_profile::ModulePrefixRewrites::On {
        definitely_typed: true,
        deep_import_peel: true,
        decline_bare_directory_match: true,
    },
    workspace_packages: true,
    // Declaration merging: interface + variable under one qname.
    overload_pick_all: true,
    argument_dependent_lookup: false,
    associated_type_projection: false,
    blanket_impl_resolution: false,
    // jest/vitest globals, jQuery `$`, DOM constructors, core-lib utility types.
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::On {
        instantiate_accepts_variable: true,
    },
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    namespaceless_global_type_lookup: false,
    explicit_member_import: false,
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
