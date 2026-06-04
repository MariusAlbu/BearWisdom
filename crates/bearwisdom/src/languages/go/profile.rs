// =============================================================================
// languages/go/profile.rs — LanguageProfile for Go.
//
// Phase 6 wave-A migration. Go's structural typing is the only
// non-Explicit supertype_discovery in wave A — interfaces are satisfied
// implicitly by method-set match.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const GO_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[SymbolKind::Function, SymbolKind::Method],
    ),
    // Go has no `extends` — type embedding looks like `Inherits` in the
    // extractor but the surface form is different. Permissive listing
    // covers both interpretations.
    (EdgeKind::Inherits, &[SymbolKind::Struct, SymbolKind::Interface]),
    (EdgeKind::Implements, &[SymbolKind::Interface]),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Struct,
            SymbolKind::Interface,
            SymbolKind::TypeAlias,
            SymbolKind::Enum,
        ],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Struct]),
];

const GO_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("string", PrimKind::Str),
    ("int", PrimKind::Int),
    ("int8", PrimKind::Int),
    ("int16", PrimKind::Int),
    ("int32", PrimKind::Int),
    ("int64", PrimKind::Int),
    ("uint", PrimKind::Int),
    ("uint8", PrimKind::Int),
    ("uint16", PrimKind::Int),
    ("uint32", PrimKind::Int),
    ("uint64", PrimKind::Int),
    ("uintptr", PrimKind::Int),
    ("byte", PrimKind::Int),
    ("rune", PrimKind::Int),
    ("float32", PrimKind::Float),
    ("float64", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("error", PrimKind::Unknown),
    ("any", PrimKind::Unknown),
];

pub const GO_PROFILE: LanguageProfile = LanguageProfile {
    id: "go",
    qname_separator: ".",
    // Go has no `self`/`this`; methods take an explicit receiver parameter.
    // The extractor records the receiver's name on the method's scope
    // path; the chain walker doesn't need a keyword.
    self_keywords: &[],
    // The hallmark of Go: structural interface satisfaction.
    supertype_discovery: SupertypeDiscovery::Structural,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    // Go generics arrived in 1.18.
    has_generics: true,
    has_sum_types: false,
    look_through_optional: false,
    literal_narrowing: false,
    // Go has goroutines + channels, not value-wrapping futures. No async
    // wrapper types the engine should peel.
    async_wrappers: &[],
    // Range loops use the type-side `range` keyword, not a method call.
    // Engine iteration peeling stays off.
    iterator_method: None,
    primitive_mapping: GO_PRIMITIVES,
    kind_compatible_table: GO_KIND_TABLE,
    // An import names a package; members are keyed under its short name
    // (`gin.NewRouter`). A bare member ref whose qualifier the extractor
    // dropped resolves under `{import short name}.{target}`.
    chain_qualification: ChainQualification::PackageShortName,
    // Go has no `new` operator at the surface form the engine recognises.
    // Construction is `Foo{}` / `&Foo{}` / `make(...)` — extractor emits
    // these as Construction segments without needing a profile pattern.
    builtin_skip: None,
    namespace_decline: None,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    package_by_directory: false,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["//"],
    // Go has no visibility keywords — uppercase identifier = exported, the
    // extractor surfaces that as Visibility::Public on emission.
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
