// =============================================================================
// languages/python/profile.rs — LanguageProfile for Python.
//
// Phase 6 wave-A migration. Single source of truth for Python-specific
// type-system behaviour the engine consumes. Filled per doc 3
// (LanguageProfile spec).
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable,
    LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

/// Python's edge × symbol-kind compatibility matrix. Permissive on Calls
/// (Python's dynamic dispatch happily calls properties, classes, methods,
/// even instances if `__call__` is defined). TypeRef restricted to actual
/// type-defining kinds. Implements unused in practice (Python uses
/// duck-typing); listed for completeness.
const PY_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Class,
            SymbolKind::Constructor,
            SymbolKind::Variable,
            SymbolKind::Property,
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
        ],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class]),
];

const PY_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("str", PrimKind::Str),
    ("bytes", PrimKind::Bytes),
    ("int", PrimKind::Int),
    ("float", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("None", PrimKind::Unit),
    ("NoneType", PrimKind::Unit),
    ("Any", PrimKind::Unknown),
];

/// Python profile.
pub const PYTHON_PROFILE: LanguageProfile = LanguageProfile {
    id: "python",
    qname_separator: ".",
    self_keywords: &["self", "cls"],
    // Python uses explicit inheritance (`class Admin(User):`). The engine's
    // Explicit discovery reads Inherits refs straight from the extractor.
    supertype_discovery: SupertypeDiscovery::Explicit,
    // Multiple inheritance resolves by C3 (Python's MRO), so an asymmetric
    // diamond picks the same override the interpreter would.
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::C3,
    // pip site-packages / typeshed contribute external types.
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    // typing.Generic + PEP 585 list[int] / dict[str, V] forms.
    has_generics: true,
    // Python has typing.Union but engine-level sum-type semantics need
    // structural narrowing the runtime never enforces; conservative off.
    has_sum_types: false,
    // Optional[T] = Union[T, None]; engine peeling matches static-type
    // intent even when the runtime checks rely on `is None`.
    look_through_optional: true,
    literal_narrowing: false,
    // Both stdlib (coroutines via `async def`) and asyncio.Future wrap
    // values for `await`.
    async_wrappers: &["Coroutine", "Awaitable", "Future", "Task"],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: Some("__iter__"),
    primitive_mapping: PY_PRIMITIVES,
    kind_compatible_table: PY_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    // Phase 6 wave-A diagnosis: engine-primary on vs off produced
    // identical rates on python-black (91.86%), confirming the engine
    // doesn't regress Python chain resolution today. The 0.5pp gap vs
    // baseline 92.38% was the extractor adding 67 new refs in
    // tests/data/cases/ test corpus (intentionally weird Python that
    // black formats) — most unresolvable by design. Baseline updated to
    // reflect new extraction; engine-primary safe.
    builtin_skip: None,
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    wildcard_builtins: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    // A module-carrying ref binds by anchor: a relative `.foo`/`..bar` module
    // resolves via `in_module_from` and binds the bare name there; an absolute
    // `models`-style module maps to a directory and accepts any kind-compatible
    // file under it (`models.TextChoices` at `.../models/enums.py`).
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::On(
        crate::type_checker::profile::language_profile::ModuleAnchorBind::NameExactKind,
    ),
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::DotPrefix,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    module_scope: crate::type_checker::profile::language_profile::ModuleScope::Off,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
    file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
    alias_module_qname: false,
    module_prefix_rewrites:
        crate::type_checker::profile::language_profile::ModulePrefixRewrites::Off,
    workspace_packages: false,
    reexport_barrel_stems: &["index"],
    self_package_root: None,
    wildcard_workspace_scope: false,
    overload_pick_all: false,
    argument_dependent_lookup: false,
    associated_type_projection: false,
    blanket_impl_resolution: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::Off,
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    namespaceless_global_type_lookup:
        crate::type_checker::profile::language_profile::NamespaceScope::Off,
    explicit_member_import: false,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[ConstructorPattern::CallableClass],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AtPrefix),
    doc_comment_kinds: &["\"\"\""],
    // Python has no `public`/`private` keywords — leading underscore is the
    // convention; the extractor encodes that as visibility, the engine
    // doesn't need keyword recognition.
    visibility_keywords: &[],
    function_prototype_types: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
