// Minimal LanguageProfile for Jinja. Templating language; no chains.

use crate::type_checker::profile::language_profile::{
    CandidateDirs, ChainQualification, DispatchAxis, ImportResolution, LanguageProfile, StemMatch,
    SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};

/// `{% extends "X" %}` / `{% include "X" %}` / `{% import "X" %}` /
/// `{% from "X" %}` template-path resolution. `X` is a path relative to the
/// referencing template's directory; the extractor strips the extension at
/// extract time, so the candidate generation re-appends each Jinja extension.
/// The candidate file's single file-stem `class` host is the template
/// regardless of its name (`AnyClassInFile`).
const JINJA_IMPORTS: ImportResolution = ImportResolution {
    extensions: &["j2", "jinja", "jinja2"],
    candidate_dirs: CandidateDirs::SelfDir,
    index_files: &[],
    underscore_variant: false,
    kebab_variant: false,
    decline_leading_slash: false,
    stem_match: StemMatch::AnyClassInFile,
    bind_kind: "class",
    strategy_tag: "jinja_template_path",
};

pub const JINJA_PROFILE: LanguageProfile = LanguageProfile {
    id: "jinja",
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
    import_resolution: Some(JINJA_IMPORTS),
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    package_by_directory: false,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["{#"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
