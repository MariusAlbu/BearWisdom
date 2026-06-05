// LanguageProfile for JavaScript (when claimed by the dedicated JS
// plugin rather than the TS plugin). Mirrors the TS shape but without
// type-system specifics — JS doesn't carry type annotations.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const JS_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Variable,
            SymbolKind::Property,
            SymbolKind::Class,
            SymbolKind::Constructor,
        ],
    ),
    (EdgeKind::Inherits, &[SymbolKind::Class]),
    (EdgeKind::TypeRef, &[SymbolKind::Class]),
    (EdgeKind::Instantiates, &[SymbolKind::Class]),
];

const JS_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("string", PrimKind::Str),
    ("number", PrimKind::Float),
    ("bigint", PrimKind::Int),
    ("boolean", PrimKind::Bool),
    ("symbol", PrimKind::Symbol),
    ("undefined", PrimKind::Unit),
    ("null", PrimKind::Unit),
];

pub const JAVASCRIPT_PROFILE: LanguageProfile = LanguageProfile {
    id: "javascript",
    qname_separator: ".",
    self_keywords: &["this"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Promise"],
    // JS carries no type annotations, so a value rarely acquires an `Apply`
    // element type for the projection to read — left empty pending inference
    // that types JS containers structurally.
    container_accessors: &[],
    single_inner_wrappers: &[],
    iterator_method: None,
    primitive_mapping: JS_PRIMITIVES,
    kind_compatible_table: JS_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::FromModuleField,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::On(
        crate::type_checker::profile::language_profile::ModuleAnchorBind::NameExactKind,
    ),
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::DotSlashPrefix,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    package_by_directory: false,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
    file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
    alias_module_qname: false,
    module_prefix_rewrites: crate::type_checker::profile::language_profile::ModulePrefixRewrites::On {
        definitely_typed: true,
        deep_import_peel: true,
        decline_bare_directory_match: true,
    },
    workspace_packages: true,
    overload_pick_all: true,
    argument_dependent_lookup: false,
    associated_type_projection: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::On {
        npm_confidence: 0.85,
        lib_confidence: 0.85,
        instantiate_accepts_variable: true,
    },
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    namespaceless_global_type_lookup: false,
    explicit_member_import: false,
    constructor_patterns: &[ConstructorPattern::New, ConstructorPattern::CallableClass],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["/**"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
