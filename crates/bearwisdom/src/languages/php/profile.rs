// =============================================================================
// languages/php/profile.rs — LanguageProfile for PHP.
//
// Engine-side type-system data for PHP.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable,
    LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const PHP_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Constructor,
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
            SymbolKind::Trait,
        ],
    ),
    (EdgeKind::Instantiates, &[SymbolKind::Class]),
];

const PHP_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("string", PrimKind::Str),
    ("int", PrimKind::Int),
    ("integer", PrimKind::Int),
    ("float", PrimKind::Float),
    ("double", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("boolean", PrimKind::Bool),
    ("void", PrimKind::Unit),
    ("null", PrimKind::Unit),
    ("mixed", PrimKind::Unknown),
    ("never", PrimKind::Never),
];

/// Convert a PHP namespace import to the slash-delimited form used only while
/// comparing that import with an indexed file path. The extracted module stays
/// raw (`Illuminate\\Database`) so resolver diagnostics and other import
/// strategies retain the source spelling.
pub(crate) fn normalize_php_module_path_for_match(module: &str) -> String {
    module.replace('\\', "/")
}

pub(crate) fn php_module_path_match(
    module: &str,
) -> crate::type_checker::profile::language_profile::ModulePathMatch {
    crate::type_checker::profile::language_profile::ModulePathMatch {
        module_path: normalize_php_module_path_for_match(module),
        path_variants: Vec::new(),
        required_file_prefix: None,
        compound_extensions: &[],
        authority: crate::type_checker::profile::language_profile::ModuleMatchAuthority::Heuristic,
        source_module_path_policy: super::module_paths::PHP_SOURCE_MODULE_PATH_POLICY,
    }
}

/// Build the exact qualified-name spellings an imported PHP namespace can
/// denote in the symbol index.
///
/// PHP source keeps namespace components separated by `\\`, while indexed
/// containment places a nested type below its owner with `.`. For
/// `use Illuminate\\Database\\Eloquent; Eloquent\\Builder::query()`, the
/// source spelling is `Illuminate\\Database\\Eloquent\\Builder` and the
/// canonical containment spelling is `Illuminate\\Database\\Eloquent.Builder`.
/// Empty namespace components are rejected so malformed source text never
/// widens a resolver lookup.
pub(crate) fn php_qualified_import_type_candidates(
    module: &str,
    imported_name: &str,
    qualified_tail: &str,
) -> Vec<String> {
    if !php_namespace_path_is_well_formed(module)
        || !php_namespace_path_is_well_formed(imported_name)
        || !php_namespace_path_is_well_formed(qualified_tail)
    {
        return Vec::new();
    }

    let source_qname = format!("{module}\\{imported_name}\\{qualified_tail}");
    let Some((owner, type_name)) = source_qname.rsplit_once('\\') else {
        return Vec::new();
    };
    let containment_qname = format!("{owner}.{type_name}");
    vec![source_qname, containment_qname]
}

pub(super) fn php_namespace_path_is_well_formed(path: &str) -> bool {
    !path.is_empty() && path.split('\\').all(|segment| !segment.is_empty())
}

/// PHP profile.
pub const PHP_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    implicit_prelude_namespaces: &[],
    compiled_name_prefixes: &[],
    id: "php",
    qname_separator: "\\",
    member_chain_markers: &["->", "::"],
    declaration_merging: crate::type_checker::profile::language_profile::MergeScope::None,
    receiver_spellings: &[
        crate::type_checker::profile::language_profile::ReceiverSpelling::enclosing("$this", "->"),
        crate::type_checker::profile::language_profile::ReceiverSpelling::enclosing("self", "::"),
        crate::type_checker::profile::language_profile::ReceiverSpelling::enclosing("static", "::"),
        crate::type_checker::profile::language_profile::ReceiverSpelling::parent("parent", "::"),
    ],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: true,
    look_through_optional: true,
    reference_member_projection: false,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: PHP_PRIMITIVES,
    kind_compatible_table: PHP_KIND_TABLE,
    // Same-namespace + `use`-statement qualification of a bare receiver type
    // via the structured walker's `qualify_current_ty`.
    chain_qualification: ChainQualification::SamePackageAndImportsWithQualifiedRoot(
        crate::type_checker::profile::language_profile::QualifiedImportRoot {
            module_path_adapter: php_module_path_match,
            type_candidates: php_qualified_import_type_candidates,
        },
    ),
    builtin_skip: Some(super::predicates::is_php_builtin),
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
    constructor_patterns: &[ConstructorPattern::New],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AttrBracket),
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
