// LanguageProfile for C/C++ in shadow mode.

use super::predicates;
use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const C_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        // Variable is callable in C through two real shapes: a function pointer
        // invoked through its binding (`void (*fp)(void); fp();`, extracted as a
        // Variable) and an object-like `#define` that aliases a callable
        // (`#define ngx_free free`, emitted as a Variable by the preproc pusher).
        // A non-callable `int x; x();` is a compile error, so admitting Variable
        // here binds real call edges without inventing false ones.
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
            SymbolKind::Variable,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class, SymbolKind::Struct]),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Struct],
    ),
];

const C_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("char", PrimKind::Str),
    ("int", PrimKind::Int),
    ("short", PrimKind::Int),
    ("long", PrimKind::Int),
    ("size_t", PrimKind::Int),
    ("ssize_t", PrimKind::Int),
    ("float", PrimKind::Float),
    ("double", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("void", PrimKind::Unit),
];

pub const C_LANG_PROFILE: LanguageProfile = LanguageProfile {
    id: "c",
    qname_separator: "::",
    self_keywords: &["this"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: false,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: C_PRIMITIVES,
    kind_compatible_table: C_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    // Template parameters (`T`, `U`, `_Range`, `<...>`, `this`, `nullptr`, …)
    // are not project symbols; decline them before the bare-name ladder so they
    // are never bound to a same-named project symbol or seeded as a chain miss.
    builtin_skip: Some(predicates::is_template_param),
    // R-package native C sources: a `.C("LENGTH")` callee names the R C API,
    // not a same-named project symbol. Declined before the ladder only when
    // the file carries the R-package namespace; external classification then
    // brands it `r.c.api`.
    namespace_decline: Some(
        crate::type_checker::profile::language_profile::NamespaceDecline {
            file_namespace: super::predicates::R_PACKAGE_SENTINEL,
            is_reserved: predicates::is_r_c_api_symbol,
        },
    ),
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    wildcard_builtins: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
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
    overload_pick_all: false,
    // C++ argument-dependent lookup: a bare `swap(a, b)` resolves to a free
    // function `swap` declared in the namespace of an argument's type.
    argument_dependent_lookup: true,
    associated_type_projection: false,
    blanket_impl_resolution: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::Off,
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    // C/C++ functions are project-global (a cross-file call to a `static` is a
    // compile error, so the dead-last rung only ever binds unique externs).
    // Recovers cross-translation-unit calls (redis `sdsfree`, nginx `ngx_*`).
    namespaceless_global_type_lookup:
        crate::type_checker::profile::language_profile::NamespaceScope::Global,
    explicit_member_import: false,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["//", "/*", "/**"],
    visibility_keywords: &[
        ("public", Visibility::Public),
        ("private", Visibility::Private),
        ("protected", Visibility::Protected),
    ],
    function_prototype_types: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
