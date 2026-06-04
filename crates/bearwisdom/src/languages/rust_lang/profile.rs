// =============================================================================
// languages/rust_lang/profile.rs — LanguageProfile for Rust.
//
// Engine-side type-system data for Rust. Registered via
// `RustLangPlugin::profile()` so the engine builds its MembersIndex,
// SupertypeGraph, and SymbolTypeMap from Rust extraction output.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable, LanguageProfile,
    SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

/// Kind-compatibility table for Rust.
///
///   - Calls: function (free fn), method (impl block), constructor
///     (`Foo::new` and ergonomic factories the extractor tags), variable
///     (callable closures bound to lets), parameter (closures via
///     `impl Fn(...)` param types).
///   - Inherits: trait (Rust has trait subtyping; structs themselves don't
///     extend other structs).
///   - Implements: trait (only valid impl target).
///   - TypeRef: struct / enum / trait / type_alias.
///   - Instantiates: struct / enum (enum-variant construction via the
///     extractor's Construction tag).
const RUST_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
            SymbolKind::Variable,
            SymbolKind::Parameter,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Trait]),
    (EdgeKind::Implements, &[SymbolKind::Trait]),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::Trait,
            SymbolKind::TypeAlias,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Struct, SymbolKind::Enum],
    ),
];

const RUST_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("bool", PrimKind::Bool),
    ("char", PrimKind::Str),
    ("str", PrimKind::Str),
    ("String", PrimKind::Str),
    ("i8", PrimKind::Int),
    ("i16", PrimKind::Int),
    ("i32", PrimKind::Int),
    ("i64", PrimKind::Int),
    ("i128", PrimKind::Int),
    ("isize", PrimKind::Int),
    ("u8", PrimKind::Int),
    ("u16", PrimKind::Int),
    ("u32", PrimKind::Int),
    ("u64", PrimKind::Int),
    ("u128", PrimKind::Int),
    ("usize", PrimKind::Int),
    ("f32", PrimKind::Float),
    ("f64", PrimKind::Float),
    ("()", PrimKind::Unit),
    ("!", PrimKind::Never),
];

/// Rust profile.
pub const RUST_PROFILE: LanguageProfile = LanguageProfile {
    id: "rust",
    qname_separator: ".",
    // `self`, `Self`, and `&self` / `&mut self` — the chain extractor
    // collapses receivers to a single `self` token; engine treats it
    // uniformly. `super::` is a module-path qualifier handled by the
    // qname separator + namespace lookup, not via self_keywords.
    self_keywords: &["self", "Self"],
    // Rust's supertyping is via trait bounds, not struct extension —
    // explicit trait edges (Implements / trait `: Bound` clauses).
    supertype_discovery: SupertypeDiscovery::Explicit,
    // Trait default methods on external crates are reachable via member
    // lookup on user types that implement those traits.
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    // Rust `Option<T>` peels via `?` and pattern-matching, both already
    // handled by the chain walker's optional-aware path.
    look_through_optional: true,
    // Rust does not preserve integer/string literals as singleton types
    // in surface types — literal-narrowing stays off.
    literal_narrowing: false,
    // Async fn returns impl Future<Output = T>; await unwraps to T.
    async_wrappers: &["Future", "Pin"],
    iterator_method: Some("next"),
    primitive_mapping: RUST_PRIMITIVES,
    kind_compatible_table: RUST_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    // Shadow mode: engine builds its indexes from Rust extraction output
    // for future use, but the legacy `RustResolver` retains the resolution
    // slot. Flip after recapture-validating ±0.1pp on representative
    // Rust baselines (bw self-host + tests).
    // `Foo::new(...)`, `Foo::build(...)`, and turbofish factories. The
    // generic NamedFactory pattern catches `*::new` shapes the extractor
    // tags as Construction segments.
    builtin_skip: None,
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    constructor_patterns: &[
        ConstructorPattern::TypeColonColonNew,
        ConstructorPattern::TypeColonColonBuild,
    ],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::HashBracket),
    doc_comment_kinds: &["///", "//!"],
    visibility_keywords: &[
        ("pub", Visibility::Public),
        ("pub(crate)", Visibility::Internal),
        ("pub(super)", Visibility::Protected),
    ],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
