// =============================================================================
// languages/swift/profile.rs — LanguageProfile for Swift.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable,
    LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const SWIFT_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class]),
    (
        EdgeKind::Implements,
        &[SymbolKind::Interface, SymbolKind::Trait],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Struct,
            SymbolKind::Interface,
            SymbolKind::Trait,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Struct, SymbolKind::Enum],
    ),
];

const SWIFT_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("Int", PrimKind::Int),
    ("Int8", PrimKind::Int),
    ("Int16", PrimKind::Int),
    ("Int32", PrimKind::Int),
    ("Int64", PrimKind::Int),
    ("UInt", PrimKind::Int),
    ("UInt8", PrimKind::Int),
    ("UInt16", PrimKind::Int),
    ("UInt32", PrimKind::Int),
    ("UInt64", PrimKind::Int),
    ("Float", PrimKind::Float),
    ("Double", PrimKind::Float),
    ("Bool", PrimKind::Bool),
    ("String", PrimKind::Str),
    ("Character", PrimKind::Str),
    ("Void", PrimKind::Unit),
    ("Never", PrimKind::Never),
    ("Any", PrimKind::Unknown),
    ("AnyObject", PrimKind::Unknown),
];

pub const SWIFT_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    implicit_prelude_namespaces: &[],
    compiled_name_prefixes: &[],
    id: "swift",
    qname_separator: ".",
    declaration_merging: crate::type_checker::profile::language_profile::MergeScope::None,
    self_keywords: &["self", "Self", "super"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: true,
    reference_member_projection: false,
    literal_narrowing: false,
    async_wrappers: &["Task", "AsyncSequence"],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: Some("makeIterator"),
    primitive_mapping: SWIFT_PRIMITIVES,
    kind_compatible_table: SWIFT_KIND_TABLE,
    chain_qualification: ChainQualification::None,
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
        module_scope:
            crate::type_checker::profile::language_profile::ModuleScope::SourcesTargetSubtree,
        // Swift has no `namespace` node — a declaration's qname never carries its
        // module as a prefix, so `QnameUnder` can never match a whole-module
        // import. `Foundation`/`UIKit` externals are indexed one file per module
        // (`swift_foundation.rs`), so the module name IS the declaring file's
        // stem — the same shape OCaml/Pascal match on.
        wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::FileStem {
            underscore_prefix: false,
        },
        // Whole-module wildcard-ness is carried per-ref by `push_import`
        // (target "*" for a single-segment import), not by this blanket axis:
        // Swift also has the declaration-specific `import struct Module.Symbol`
        // form, which this flag cannot distinguish from a whole-module import.
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
    explicit_member_import: true,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[ConstructorPattern::CallableClass],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AtPrefix),
    doc_comment_kinds: &["///"],
    visibility_keywords: &[
        ("public", Visibility::Public),
        ("open", Visibility::Public),
        ("private", Visibility::Private),
        ("fileprivate", Visibility::Private),
        ("internal", Visibility::Internal),
    ],
    function_prototype_types: &[],
    external_contract_reduction: true,
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
