// LanguageProfile for SQL. Declarative DDL with no imports, namespace, or
// scope: it emits only `TypeRef` refs and binds table/type names cross-file by
// flat-global first-match (`namespaceless_global_type_lookup`). Built-in scalar
// and pseudo-function names decline before the ladder via `builtin_skip`.

use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

// SQL emits only `TypeRef` refs (FK `REFERENCES`, `ALTER TABLE` target,
// `CREATE INDEX` → table, custom column type). Table/type targets land as
// `struct`/`class`; user-defined functions as `function`; `CREATE INDEX`
// targets as `variable`.
const SQL_KIND_TABLE: KindTable = &[
    (
        EdgeKind::TypeRef,
        &[
            SymbolKind::Struct,
            SymbolKind::Class,
            SymbolKind::Function,
            SymbolKind::Variable,
        ],
    ),
    (EdgeKind::Calls, &[SymbolKind::Function, SymbolKind::Method]),
];

pub const SQL_PROFILE: LanguageProfile = LanguageProfile {
    id: "sql",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: false,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: &[],
    kind_compatible_table: SQL_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: Some(super::keywords::is_sql_builtin_type),
    namespace_decline: None,
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
    // SQL identifiers are case-insensitive per ANSI — a table/type reference
    // written in any casing binds to a same-name candidate.
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
    argument_dependent_lookup: false,
    associated_type_projection: false,
    blanket_impl_resolution: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::Off,
    namespaceless_global_type_lookup:
        crate::type_checker::profile::language_profile::NamespaceScope::Global,
    explicit_member_import: false,
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["--", "/*"],
    visibility_keywords: &[],
    function_prototype_types: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
