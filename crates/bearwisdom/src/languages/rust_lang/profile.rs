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
///     `impl Fn(...)` param types), test (a `#[test]` fn called by another
///     test), enum_member (tuple/unit-variant construction via call syntax —
///     `Some(x)`, `Ok(y)`, a `Status::Active` unit variant called as a fn
///     pointer).
///   - Inherits: trait (Rust has trait subtyping; structs themselves don't
///     extend other structs).
///   - Implements: trait (only valid impl target).
///   - TypeRef: struct / enum / trait / type_alias / enum_member (a
///     `Self::Variant` member-walk lands the variant on a TypeRef edge).
///   - Instantiates: struct / enum / enum_member (enum-variant construction).
const RUST_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
            SymbolKind::Variable,
            SymbolKind::Parameter,
            SymbolKind::Test,
            SymbolKind::EnumMember,
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
            SymbolKind::EnumMember,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Struct, SymbolKind::Enum, SymbolKind::EnumMember],
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
    // Rust's source separator is `::`. The symbol-index qname join is the
    // universal `.` (`helpers::qualify` builds `Bar.foo`), so the scope /
    // module-anchor probes run both joins — `.` matches the dotted index and
    // `::` matches the `::`-form a ref's `module` and chain prefixes carry
    // (`crate::db`). The probe set is deduped to the two joins by the engine.
    qname_separator: "::",
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
    // No target-name decline list: Rust has no bare runtime-global identifiers
    // the binder must skip — the prelude resolves through the stdlib ambient
    // path, and the generic-param / turbofish noise is dropped at extraction.
    builtin_skip: None,
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    // A qualified call (`DbPool::new()`) carries the importing module path on
    // `r.module` in its verbatim `::` form (`crate::db`). `ByNameUnderModuleDir`
    // maps the path separators to `/`, probes the `{module}{sep}{target}` qname
    // under both joins, and falls back to the module leaf (`db`) against the
    // file stems of `by_name(new)` candidates. Non-terminal: a miss falls
    // through to the scope / import / qname binders.
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::On(
        crate::type_checker::profile::language_profile::ModuleAnchorBind::ByNameUnderModuleDir,
    ),
    module_anchor_terminal: false,
    // `None`: a Rust ref's `module` is a crate-rooted `::` path with no
    // relative/absolute split at this layer, so every module runs the
    // configured `ByNameUnderModuleDir` bind.
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
