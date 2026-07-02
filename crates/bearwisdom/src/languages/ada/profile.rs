// LanguageProfile for Ada in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const ADA_KIND_TABLE: KindTable = &[
    // Ada's parens-everywhere syntax means Calls covers procedure/function
    // calls, type conversions (`UInt16(x)`), array indexing (`This.CCER(Ch)`),
    // and generic instantiation. Accept the type/value kinds so the generic
    // resolver can attribute them — what the former hand-written resolver did
    // via predicates::kind_compatible.
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
            SymbolKind::Test,
            SymbolKind::Class,
            SymbolKind::Namespace,
            SymbolKind::Variable,
            SymbolKind::Field,
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class, SymbolKind::Struct]),
    (
        EdgeKind::Implements,
        &[SymbolKind::Class, SymbolKind::Interface],
    ),
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Class,
            SymbolKind::Struct,
            SymbolKind::Interface,
            SymbolKind::Enum,
            SymbolKind::TypeAlias,
            SymbolKind::Module,
            SymbolKind::Function,
            SymbolKind::Variable,
            // Record components are the mid-chain walk-through kind for
            // `This.Port.CCER` field chains: the non-terminal `Port` hop is
            // filtered by TypeRef, so a Field receiver must be compatible.
            SymbolKind::Field,
        ],
    ),
    (
        EdgeKind::Instantiates,
        &[
            SymbolKind::Class,
            SymbolKind::Function,
            SymbolKind::Namespace,
        ],
    ),
];

const ADA_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("Integer", PrimKind::Int),
    ("Natural", PrimKind::Int),
    ("Positive", PrimKind::Int),
    ("Float", PrimKind::Float),
    ("Long_Float", PrimKind::Float),
    ("Boolean", PrimKind::Bool),
    ("Character", PrimKind::Str),
    ("String", PrimKind::Str),
];

pub const ADA_PROFILE: LanguageProfile = LanguageProfile {
    id: "ada",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: true,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
    single_inner_wrappers: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: ADA_PRIMITIVES,
    kind_compatible_table: ADA_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: Some(super::predicates::is_ada_builtin),
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &["Ada", "System", "Interfaces", "GNAT", "Standard"],
    wildcard_builtins: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::Spec(
        crate::type_checker::profile::language_profile::NormSpec {
            case_insensitive: true,
            strip_chars: &[],
            strip_prefixes: &[],
            strip_sigils: &[],
        },
    ),
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
        crate::type_checker::profile::language_profile::NamespaceScope::Global,
    explicit_member_import: false,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["--"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
