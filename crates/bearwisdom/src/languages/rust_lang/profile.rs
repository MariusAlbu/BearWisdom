// =============================================================================
// languages/rust_lang/profile.rs — LanguageProfile for Rust.
//
// Engine-side type-system data for Rust. Registered via
// `RustLangPlugin::profile()` so the engine builds its MembersIndex,
// SupertypeGraph, and SymbolTypeMap from Rust extraction output.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DerefWrapper, DispatchAxis, KindTable,
    LanguageProfile, SupertypeDiscovery,
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
///     pointer), struct (`Point { x: 1 }` struct-literal construction — the
///     extractor emits a Calls ref alongside the TypeRef at the same site, so
///     it needs the same target kind TypeRef already admits).
///   - Inherits: trait (Rust has trait subtyping; structs themselves don't
///     extend other structs).
///   - Implements: trait (only valid impl target).
///   - TypeRef: struct / enum / trait / type_alias / enum_member (a
///     `Self::Variant` member-walk lands the variant on a TypeRef edge), plus
///     method / function / field / property. The chain walker uses `TypeRef` as
///     its mid-chain walk-through filter — every non-terminal segment in
///     `c.add().finish()` is filtered against this table — so a mid-chain method
///     call (`add()`) or field access must be kind-compatible here to type the
///     receiver of the next segment. The terminal segment carries the ref's own
///     edge kind, so admitting callables here cannot loosen a real terminal
///     TypeRef ref (those target a type/variant by name, never a method).
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
            SymbolKind::Struct,
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
            SymbolKind::Method,
            SymbolKind::Function,
            SymbolKind::Field,
            SymbolKind::Property,
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
    implicit_root_types: &[],
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
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
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
    container_accessors: &[],
    // Std smart pointers that `Deref` to their single inner: the receiver of a
    // method call on `Box<C>` / `Rc<C>` / `Arc<C>` / `Pin<C>` / `Cow<'a, C>`
    // types as the inner `C`. The chain walker peels these structurally
    // (`args[0]`) at the root, then the member + trait-default walk resolves
    // against `C`. Cow's leading lifetime arg is dropped at intern time so its
    // single type arg is `args[0]`. `Vec` / `HashMap` are deliberately absent —
    // their accessors must stay on the container. `RefCell`/`Cell`/`Mutex` are
    // absent too — they expose a GUARD's members via `.borrow()`/`.lock()`, not
    // the inner's, so peeling them to the inner would be unsound.
    single_inner_wrappers: &["Box", "Rc", "Arc", "Pin", "Cow"],
    // Built-in containers whose missed member lookups retry on their Deref
    // target's member set, keeping the applied args: `Vec<T>` reheads to
    // `slice<T>` (std's inherent slice methods are qualified under `slice`),
    // `String` to `str`, `PathBuf` to `Path`. `Array` is the canonical head
    // the arena interns for the bracketed `[T]` / `[T; N]` syntax, so an
    // array/slice-typed receiver reaches the same `slice` member set. The
    // container's own members win first — `Vec::push` stays on `Vec`.
    container_deref_targets: &[
        ("Vec", "slice"),
        ("Array", "slice"),
        ("String", "str"),
        ("PathBuf", "Path"),
    ],
    // A user `impl Deref for C { type Target = Inner }` exposes Inner's members
    // on a `C` receiver. The peel reads the inner from the already-indexed
    // `field_type["C.Target"]` binding, gated on a real `C → Deref` supertype
    // edge so a bare name match never fires it.
    deref_wrapper: Some(DerefWrapper {
        trait_name: "Deref",
        target_assoc: "Target",
    }),
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
    wildcard_builtins: &[],
    import_resolution: None,
    // `FromModuleField` reads the `module` field on `EdgeKind::Imports` refs
    // (the `use crate_name::Foo` import binding), so the file context carries
    // `{imported_name: "Foo", module_path: Some("crate_name")}`. The
    // imported_namespace rule then matches a bare `Foo` TypeRef to the
    // symbol whose file path contains `crate-name/` (after hyphen→underscore
    // normalization, since Cargo uses hyphens in directory names while Rust
    // module paths use underscores).
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::FromModuleField,
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
    module_scope: crate::type_checker::profile::language_profile::ModuleScope::Off,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    namespace_imports_are_wildcards: false,
    delegate_wrappers: &[],
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
    file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
    alias_module_qname: false,
    module_prefix_rewrites:
        crate::type_checker::profile::language_profile::ModulePrefixRewrites::Off,
    // A bench/example/test target imports its own package by its published
    // crate name (`use tantivy::Index`) exactly like a sibling would — Cargo
    // has no separate "internal" import syntax for it. The workspace-package
    // scoped bind (keyed on `ProjectContext::workspace_pkg_by_declared_name`,
    // which registers the crate's own root package alongside its workspace
    // members) resolves it the same way an npm sibling-package import does.
    workspace_packages: true,
    reexport_barrel_stems: &["lib", "main"],
    // `crate::` is the crate-root path to the current package itself — a
    // `use crate::Thing;` (or an inline `crate::db::Pool` path) names this
    // file's own package, not a sibling by declared name. Resolved against
    // the ref's own `file_package_id` rather than
    // `workspace_pkg_by_declared_name`, so a re-exported name (`pub use
    // thing::Thing;` at the crate root) binds the same way a direct
    // declaration would — both are members of the same package.
    self_package_root: Some("crate"),
    // `use tantivy::collector::*;` brings every name the `collector` module
    // exposes into bare scope, including ones only reachable through a
    // `pub use` re-export from a deeper submodule — the physical declaration
    // site's qualified name carries no crate/module prefix at all, so a
    // qname-prefix test can never line it up with the glob's module path.
    // Scoped through the same package-id + file-path-substring search
    // `workspace_packages` gives an explicit import.
    wildcard_workspace_scope: true,
    overload_pick_all: false,
    argument_dependent_lookup: false,
    // `Self::Output` / `<C as Trait>::Item` return strings project through the
    // receiver's impl binding (`type Output = Concrete`, already in field_type as
    // `C.Output`) so a chain types past an associated-type-returning method.
    associated_type_projection: true,
    // `impl<U: Bound> Trait for U {}` — the blanket trait's default methods
    // become reachable from every concrete C whose supertype graph reaches
    // Bound. A second `build_explicit` pass adds the `C → Trait` edge per
    // bound-satisfying C; an unsatisfied or unhydrated bound declines.
    blanket_impl_resolution: true,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::Off,
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    namespaceless_global_type_lookup:
        crate::type_checker::profile::language_profile::NamespaceScope::Off,
    explicit_member_import: false,
    multi_candidate_ranking: false,
    scope_functions: &[],
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
    function_prototype_types: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
