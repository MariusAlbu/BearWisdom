// LanguageProfile for F# in shadow mode.

use super::predicates;
use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DecoratorSyntax, DispatchAxis, KindTable, LanguageProfile,
    SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const FSHARP_KIND_TABLE: KindTable = &[
    // Discriminated-union and enum cases (`Ok value`, `Circle`) are applied
    // like functions and extracted as `EnumMember`; accept that shape for
    // `Calls` so a bare case application binds to its declaration.
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
            SymbolKind::EnumMember,
        ],
    ),
    (
        EdgeKind::Inherits,
        &[SymbolKind::Class, SymbolKind::Interface],
    ),
    (EdgeKind::Implements, &[SymbolKind::Interface]),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
            SymbolKind::Struct,
            SymbolKind::Module,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Struct],
    ),
];

const FSHARP_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("int", PrimKind::Int),
    ("float", PrimKind::Float),
    ("double", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("string", PrimKind::Str),
    ("char", PrimKind::Str),
    ("unit", PrimKind::Unit),
];

pub const FSHARP_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    // FSharp.Core's Option and Result cases are always in scope without an
    // `open`. Option's compiled surface keeps friendly member names
    // (`FSharpOption.Some`), so the prelude rule binds them directly.
    // FSharpValueOption is deliberately absent: its compiled surface ALSO
    // exposes a `Some` method (backing the source-level `ValueSome`
    // identifier), so listing it here would make `Some` match two distinct
    // qnames and the rule's ambiguity check would decline both.
    implicit_prelude_namespaces: &[
        "Microsoft.FSharp.Core.FSharpOption",
        "Microsoft.FSharp.Core.FSharpResult",
    ],
    // Result's cases compile to `New<Case>` static factory methods instead of
    // keeping friendly names (`FSharpResult.NewOk`, not `.Ok`) — the compiler's
    // general discriminated-union encoding, which Option is hand-special-cased
    // out of. The prelude rule probes `New<target>` under the namespaces above
    // when the bare target itself isn't a direct member.
    compiled_name_prefixes: &["New"],
    id: "fsharp",
    qname_separator: ".",
    self_keywords: &["this"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Async", "Task"],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: FSHARP_PRIMITIVES,
    kind_compatible_table: FSHARP_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: Some(predicates::is_fsharp_prelude_operator),
    namespace_decline: None,
    imports: crate::type_checker::profile::language_profile::ImportAxes {
        decline_qualified_when_prefix_imported: false,
        import_resolution: None,
        // `open Foo` (and the `#r "Foo.dll"` assembly-reference form) always sets
        // `ExtractedRef::module`, so the module-field path carries every import
        // without a separate target-echo.
        import_module_path:
            crate::type_checker::profile::language_profile::ImportModulePath::FromModuleField,
        module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
        module_anchor_terminal: false,
        relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
        external_by_import: None,
        module_scope: crate::type_checker::profile::language_profile::ModuleScope::Off,
        wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
        // `open Namespace` has no named-member form in F# — every `open` brings
        // every direct member of the namespace into bare scope, so it is always
        // a wildcard.
        namespace_imports_are_wildcards: true,
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
    },
    module_skip: None,
    ambient_namespace_prefixes: &[],
    wildcard_builtins: &[],
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    delegate_wrappers: &[],
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
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AttrBracket),
    doc_comment_kinds: &["///"],
    visibility_keywords: &[],
    function_prototype_types: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
