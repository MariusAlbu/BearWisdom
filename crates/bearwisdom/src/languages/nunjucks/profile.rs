use crate::type_checker::profile::language_profile::{
    CandidateDirs, ChainQualification, DispatchAxis, ImportModulePath, ImportResolution,
    LanguageProfile, StemMatch, SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};

/// `{% extends %}` / `{% include %}` resolution. The raw target is a template
/// path; when it carries no `.njk` / `.nunjucks` / `.html` / `.htm` extension
/// the candidate set appends the `.njk` / `.nunjucks` / `.html` forms. A target
/// already ending in `.htm` is taken verbatim (the verbatim base candidate
/// covers it) even though `.htm` is not in the appended set. The binding symbol
/// is the candidate file's stem-named class.
const NUNJUCKS_IMPORTS: ImportResolution = ImportResolution {
    extensions: &["njk", "nunjucks", "html"],
    candidate_dirs: CandidateDirs::SelfDir,
    index_files: &[],
    underscore_variant: false,
    kebab_variant: false,
    decline_leading_slash: false,
    stem_match: StemMatch::StemExact,
    bind_kind: "class",
    strategy_tag: "nunjucks_partial",
};

pub const NUNJUCKS_PROFILE: LanguageProfile = LanguageProfile {
    id: "nunjucks",
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
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    import_resolution: Some(NUNJUCKS_IMPORTS),
    import_module_path: ImportModulePath::EchoTarget,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    package_by_directory: false,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
    file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["{#"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
