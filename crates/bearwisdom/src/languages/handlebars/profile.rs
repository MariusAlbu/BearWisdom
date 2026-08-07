// Minimal LanguageProfile for Handlebars. Templating language; no chains.

use crate::type_checker::profile::language_profile::{
    CandidateDirs, ChainQualification, DispatchAxis, ImportModulePath, ImportResolution,
    LanguageProfile, StemMatch, SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};

/// `{{> partial}}` resolution. The raw target is a bare stem (possibly
/// camelCase, possibly with a relative dir prefix); extensions cover the
/// Handlebars / Mustache family. Candidates are probed in the source dir and
/// up the directory tree under the conventional partials subdirectories, with
/// an `_{stem}` partial-file sibling and a kebab-cased name variant. The
/// binding symbol is the candidate file's stem-named (or underscore-stripped
/// stem-named) class.
const HANDLEBARS_IMPORTS: ImportResolution = ImportResolution {
    extensions: &["hbs", "handlebars", "mustache", "html"],
    candidate_dirs: CandidateDirs::WalkUp {
        dirs: &["partials", "_partials", "_includes", "templates"],
        depth: 4,
    },
    index_files: &[],
    underscore_variant: true,
    kebab_variant: true,
    decline_leading_slash: false,
    stem_match: StemMatch::StemOrUnderscoreStripped,
    bind_kind: "class",
    strategy_tag: "handlebars_partial",
};

pub const HANDLEBARS_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    id: "handlebars",
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
    kind_compatible_table: PERMISSIVE_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    wildcard_builtins: &[],
    import_resolution: Some(HANDLEBARS_IMPORTS),
    import_module_path: ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    module_scope: crate::type_checker::profile::language_profile::ModuleScope::Off,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    namespace_imports_are_wildcards: false,
    delegate_wrappers: &[],
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
        crate::type_checker::profile::language_profile::NamespaceScope::Off,
    explicit_member_import: false,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["{{!"],
    visibility_keywords: &[],
    function_prototype_types: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
