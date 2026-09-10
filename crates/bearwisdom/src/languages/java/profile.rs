// =============================================================================
// languages/java/profile.rs — LanguageProfile for Java.
//
// Phase 6 wave-A migration.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DelegateShape, DispatchAxis,
    KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const JAVA_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Method,
            SymbolKind::Function,
            SymbolKind::Constructor,
        ],
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
    implicit_root_types: &[],
    implicit_prelude_namespaces: &["java.lang"],
    compiled_name_prefixes: &[],
    id: "java",
    qname_separator: ".",
    declaration_merging: crate::type_checker::profile::language_profile::MergeScope::None,
    self_keywords: &["this", "super"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
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
    reference_member_projection: false,
    literal_narrowing: false,
    async_wrappers: &["CompletableFuture", "Future"],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: Some("iterator"),
    primitive_mapping: JAVA_PRIMITIVES,
    kind_compatible_table: JAVA_KIND_TABLE,
    // Members are keyed under package-qualified qnames; a bare receiver
    // (`Repository`, or a same-package return type `Entity`) qualifies via
    // its package then explicit imports before member lookup.
    chain_qualification: ChainQualification::SamePackageAndImports,
    // Engine-primary validated post java.lang pre-pull (commit 0548421a).
    // java-spring-petclinic gate: 92.19% > 83.26% baseline (+8.93pp).
    // The earlier -2.68pp regression was a JDK demand-walker gap (bare
    // `String` refs never triggering a pull), not the engine.
    builtin_skip: None,
    namespace_decline: None,
    imports: crate::type_checker::profile::language_profile::ImportAxes {
        decline_qualified_when_prefix_imported: false,
        import_resolution: None,
        import_module_path:
            crate::type_checker::profile::language_profile::ImportModulePath::FromModuleField,
        module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
        module_anchor_terminal: false,
        relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
        external_by_import: None,
        module_scope: crate::type_checker::profile::language_profile::ModuleScope::Off,
        wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
        namespace_imports_are_wildcards: false,
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
    // Generic JDK callback interfaces. Their callback-parameter positions are
    // recoverable entirely from their generic arguments. Interfaces with a
    // fixed primitive or repeated generic parameter position (for example
    // IntConsumer or BinaryOperator) are intentionally absent because the
    // generic slots cannot describe their complete callback inputs.
    delegate_wrappers: &[
        ("java.util.function.Consumer", DelegateShape::AllParams),
        ("java.util.function.BiConsumer", DelegateShape::AllParams),
        ("java.util.function.Function", DelegateShape::LastIsReturn),
        ("java.util.function.BiFunction", DelegateShape::LastIsReturn),
        ("java.util.function.Predicate", DelegateShape::AllParams),
        ("java.util.function.BiPredicate", DelegateShape::AllParams),
        ("java.util.function.Supplier", DelegateShape::LastIsReturn),
        // UnaryOperator<T> has one callback parameter of T. BinaryOperator<T>
        // needs T twice, which the two generic-slot shapes cannot express.
        ("java.util.function.UnaryOperator", DelegateShape::AllParams),
        ("java.util.function.ToIntFunction", DelegateShape::AllParams),
        (
            "java.util.function.ToLongFunction",
            DelegateShape::AllParams,
        ),
        (
            "java.util.function.ToDoubleFunction",
            DelegateShape::AllParams,
        ),
        (
            "java.util.function.ToIntBiFunction",
            DelegateShape::AllParams,
        ),
        (
            "java.util.function.ToLongBiFunction",
            DelegateShape::AllParams,
        ),
        (
            "java.util.function.ToDoubleBiFunction",
            DelegateShape::AllParams,
        ),
    ],
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
    constructor_patterns: &[ConstructorPattern::New],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AtPrefix),
    doc_comment_kinds: &["/**"],
    visibility_keywords: &[
        ("public", Visibility::Public),
        ("private", Visibility::Private),
        ("protected", Visibility::Protected),
    ],
    function_prototype_types: &[],
    external_contract_reduction: true,
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
