// =============================================================================
// languages/python/profile.rs — LanguageProfile for Python.
//
// Phase 6 wave-A migration. Single source of truth for Python-specific
// type-system behaviour the engine consumes. Filled per doc 3
// (LanguageProfile spec).
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable, LanguageProfile,
    SupertypeDiscovery,
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
    constructor_patterns: &[ConstructorPattern::CallableClass],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AtPrefix),
    doc_comment_kinds: &["\"\"\""],
    // Python has no `public`/`private` keywords — leading underscore is the
    // convention; the extractor encodes that as visibility, the engine
    // doesn't need keyword recognition.
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
