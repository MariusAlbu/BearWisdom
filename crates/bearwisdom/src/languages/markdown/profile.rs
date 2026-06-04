use crate::type_checker::profile::language_profile::{
    CandidateDirs, ChainQualification, DispatchAxis, ImportModulePath, ImportResolution,
    LanguageProfile, StemMatch, SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};

/// Relative-link resolution. The link target is joined to the source dir and
/// lexically normalized; when it carries no markdown extension the candidate
/// set appends each markdown-family extension, and in every case probes the
/// `index` / `README` directory-entry forms (a link to a directory resolves to
/// its index document). The binding symbol is the candidate file's stem-named
/// class. A `.french`-style translation suffix is appended-to, not replaced,
/// because it is not a markdown extension.
const MARKDOWN_IMPORTS: ImportResolution = ImportResolution {
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

pub const MARKDOWN_PROFILE: LanguageProfile = LanguageProfile {
    id: "markdown",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: false,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: &[],
    kind_compatible_table: PERMISSIVE_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    import_resolution: Some(MARKDOWN_IMPORTS),
    import_module_path: ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["<!--"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
