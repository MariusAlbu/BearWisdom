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
    id: "handlebars",
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
    import_resolution: Some(HANDLEBARS_IMPORTS),
    import_module_path: ImportModulePath::None,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["{{!"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
