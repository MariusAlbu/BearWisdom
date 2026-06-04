// Minimal LanguageProfile for YAML. Data-format / config language; engine
// doesn't take the resolution slot. Registered so no plugin falls through
// to DEFAULT_PROFILE at runtime.

use crate::type_checker::profile::language_profile::{
    CandidateDirs, ChainQualification, DispatchAxis, ImportModulePath, ImportResolution,
    LanguageProfile, StemMatch, SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};

/// GitHub-Actions `uses: ./path/to/action` resolution. A target that already
/// ends in `.yml` / `.yaml` (a reusable workflow) is taken verbatim; otherwise
/// it names either a composite-action directory holding `action.{yml,yaml}` (the
/// directory-entry convention) or a sibling `{name}.{yml,yaml}` file. The
/// binding symbol is the candidate file's file-scoped class, whose name is the
/// full basename including extension (`BasenameWithExt`).
const YAML_IMPORTS: ImportResolution = ImportResolution {
    extensions: &["yml", "yaml"],
    candidate_dirs: CandidateDirs::SelfDir,
    index_files: &["action"],
    underscore_variant: false,
    kebab_variant: false,
    decline_leading_slash: false,
    stem_match: StemMatch::BasenameWithExt,
    bind_kind: "class",
    strategy_tag: "yaml_uses",
};

pub const YAML_PROFILE: LanguageProfile = LanguageProfile {
    id: "yaml",
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
    import_resolution: Some(YAML_IMPORTS),
    import_module_path: ImportModulePath::None,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["#"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
