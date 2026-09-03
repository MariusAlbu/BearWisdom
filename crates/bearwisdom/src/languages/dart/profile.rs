// =============================================================================
// languages/dart/profile.rs — LanguageProfile for Dart.
// =============================================================================

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, ConstructorPattern, DecoratorSyntax, DispatchAxis, KindTable,
    LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind, Visibility};

const DART_KIND_TABLE: KindTable = &[
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
        &[SymbolKind::Class, SymbolKind::Interface],
    ),
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

const DART_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("int", PrimKind::Int),
    ("double", PrimKind::Float),
    ("num", PrimKind::Float),
    ("bool", PrimKind::Bool),
    ("String", PrimKind::Str),
    ("void", PrimKind::Unit),
    ("Null", PrimKind::Unit),
    ("Never", PrimKind::Never),
    ("dynamic", PrimKind::Unknown),
    ("Object", PrimKind::Unknown),
];

pub const DART_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    implicit_prelude_namespaces: &[],
    compiled_name_prefixes: &[],
    id: "dart",
    qname_separator: ".",
    declaration_merging: crate::type_checker::profile::language_profile::MergeScope::None,
    self_keywords: &["this", "super"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: true,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: true,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &["Future", "Stream"],
    container_accessors: &[],
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: Some("iterator"),
    primitive_mapping: DART_PRIMITIVES,
    kind_compatible_table: DART_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    imports: crate::type_checker::profile::language_profile::ImportAxes {
        decline_qualified_when_prefix_imported: false,
        import_resolution: None,
        // A plain (unprefixed, unrestricted) `import 'uri';` is marked wildcard
        // at extract time (`target_name: "*"`); `FromModuleField` is what carries
        // that ref's `module` (the wildcard's bare library stem, or the raw URI
        // for a scoped import) onto `ImportEntry.module_path` for the ladder.
        import_module_path:
            crate::type_checker::profile::language_profile::ImportModulePath::FromModuleField,
        // Library-prefix bind: a `i0.Value` ref carries the prefix's import URI on
        // `module`. Resolve that URI to its project file via `in_module_from` and
        // bind the bare name there; on a miss, terminate so an external prefix is
        // not hijacked by a same-named local symbol.
        module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::On(
            crate::type_checker::profile::language_profile::ModuleAnchorBind::NameExactKind,
        ),
        module_anchor_terminal: true,
        relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
        external_by_import: None,
        module_scope: crate::type_checker::profile::language_profile::ModuleScope::Off,
        // Dart top-level declarations carry no namespace prefix in their qname
        // (`BuildContext`, not `widgets.BuildContext`), so `QnameUnder` can never
        // match a whole-library import. A scheme-prefixed wildcard
        // (`package:flutter/material.dart`, `dart:async`) carries its bare
        // PACKAGE identity on `module` (the extractor reduces the URI at capture
        // time) — `PackageRoot` matches that against a candidate's external
        // `ext:<lang>:<pkg>/…` package segment, so it reaches through a barrel
        // library re-exporting `src/widgets/framework.dart` to the file that
        // actually declares the member. A schemeless (relative, same-project)
        // wildcard carries the old bare file stem instead, and `PackageRoot`
        // falls back to the file-stem check for it — unchanged from before.
        wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::PackageRoot,
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
    constructor_patterns: &[ConstructorPattern::New, ConstructorPattern::CallableClass],
    class_builder_specs: &[],
    decorator_syntax: Some(DecoratorSyntax::AtPrefix),
    doc_comment_kinds: &["///"],
    visibility_keywords: &[],
    function_prototype_types: &[],
    external_contract_reduction: true,
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
