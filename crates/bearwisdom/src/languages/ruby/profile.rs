// =============================================================================
// languages/ruby/profile.rs — LanguageProfile for Ruby.
//
// Engine-side type-system data for Ruby.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const RUBY_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Method,
            SymbolKind::Function,
            SymbolKind::Constructor,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class]),
    (
        EdgeKind::Implements,
        &[SymbolKind::Module, SymbolKind::Interface],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Module,
            SymbolKind::Interface,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[SymbolKind::Class, SymbolKind::Module],
    ),
];

const RUBY_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("String", PrimKind::Str),
    ("Symbol", PrimKind::Symbol),
    ("Integer", PrimKind::Int),
    ("Fixnum", PrimKind::Int),
    ("Float", PrimKind::Float),
    ("TrueClass", PrimKind::Bool),
    ("FalseClass", PrimKind::Bool),
    ("NilClass", PrimKind::Unit),
];

/// Ruby profile.
pub const RUBY_PROFILE: LanguageProfile = LanguageProfile {
    id: "ruby",
    qname_separator: "::",
    self_keywords: &["self"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
    iterator_method: Some("each"),
    primitive_mapping: RUBY_PRIMITIVES,
    kind_compatible_table: RUBY_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    // `require`/`require_relative` anchor: resolve the require path to its
    // project file and bind the same-named module/class symbol when present,
    // else the first symbol in the file (anchors the cross-file edge).
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::On(
        crate::type_checker::profile::language_profile::ModuleAnchorBind::PreferNamedElseFirst,
    ),
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    // Gem symbols (origin='external') bind by name, gated by the file's
    // imported-gem set, at reduced confidence — the one strategy that binds
    // to externals below 1.0.
    external_by_import: Some(crate::type_checker::profile::language_profile::ExternalByImport {
        confidence: 0.8,
    }),
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
    namespaceless_global_type_lookup: false,
    constructor_patterns: &[ConstructorPattern::ClassDotNew],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["#"],
    visibility_keywords: &[
        ("public", Visibility::Public),
        ("private", Visibility::Private),
        ("protected", Visibility::Protected),
    ],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
