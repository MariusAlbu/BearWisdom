use crate::type_checker::profile::language_profile::{
    CandidateDirs, ChainQualification, DispatchAxis, ImportResolution, LanguageProfile, StemMatch,
    SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};

/// Relative-link resolution for MDX `Imports` refs — the markdown-link half of
/// MDX's ref handling. A bare path target (`api/overview`) binds to the
/// file-stem `class` host symbol of the linked `.md`/`.mdx` file. Mirrors the
/// markdown profile's link rule; JSX component refs (every non-`Imports` ref)
/// flow through the TypeScript-shaped strategies the rest of the profile drives.
const MDX_IMPORTS: ImportResolution = ImportResolution {
    extensions: &["md", "markdown", "mdown", "mkd", "mkdn", "mdx"],
    candidate_dirs: CandidateDirs::SelfDir,
    index_files: &["index", "README", "readme", "Readme"],
    underscore_variant: false,
    kebab_variant: false,
    decline_leading_slash: false,
    stem_match: StemMatch::StemExact,
    bind_kind: "class",
    strategy_tag: "markdown_relative_link",
};

pub const MDX_PROFILE: LanguageProfile = LanguageProfile {
    id: "mdx",
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
    import_resolution: Some(MDX_IMPORTS),
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::FromModuleField,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::On(
        crate::type_checker::profile::language_profile::ModuleAnchorBind::NameExactKind,
    ),
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::DotSlashPrefix,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    module_scope: crate::type_checker::profile::language_profile::ModuleScope::Off,
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
    blanket_impl_resolution: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::On {
        instantiate_accepts_variable: true,
    },
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    namespaceless_global_type_lookup: false,
    explicit_member_import: false,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["{/*"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
