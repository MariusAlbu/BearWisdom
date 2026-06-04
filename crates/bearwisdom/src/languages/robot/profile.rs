use crate::type_checker::profile::language_profile::{
    AliasDecode, ChainQualification, DispatchAxis, FileScopedImports, LanguageProfile,
    NameNormalization, NormSpec, SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};

/// Robot Framework keyword/variable name normalization. Robot treats spaces
/// and underscores as equivalent, is case-insensitive, and strips the BDD-style
/// prefixes (`Given`/`When`/`Then`/`And`/`But`) that are call-site decorators,
/// not part of a keyword's identity. The `${…}` / `@{…}` / `&{…}` sigil pairs
/// wrap a variable reference; stripping them lets `${HOST}` bind to the `HOST`
/// variable symbol. The prefix strip runs case-folded (the engine folds the
/// comparison when `case_insensitive`), so a lowercase `when ` call still
/// strips against the title-case prefix list.
const ROBOT_NAME_NORM: NormSpec = NormSpec {
    case_insensitive: true,
    strip_chars: &[' ', '_'],
    strip_prefixes: &["given ", "when ", "then ", "and ", "but "],
    strip_sigils: &[("${", "}"), ("@{", "}"), ("&{", "}")],
};

pub const ROBOT_PROFILE: LanguageProfile = LanguageProfile {
    id: "robot",
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
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    name_normalization: NameNormalization::Spec(ROBOT_NAME_NORM),
    package_by_directory: false,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
    // A `.robot` / `.resource` resource import or a Python-library import brings
    // its file's keywords/variables into bare-name scope (the wildcard-flagged
    // file imports built by `build_file_context`). The alias-decode pass binds
    // dynamic-library keywords: a `@keyword("alias")` entry carries
    // `Class::method` and binds the Python method; a `get_keyword_names` /
    // `KEYWORDS` entry carries `Class` and binds that class; a module-level
    // `KEYWORDS` dict carries no owner and falls back to the file's dispatch
    // class.
    file_scoped_imports: FileScopedImports::On {
        wildcard_only: true,
        confidence: 1.0,
        alias_decode: Some(AliasDecode {
            separator: "::",
            fallback_kind: Some("class"),
            member_confidence: 0.95,
            type_confidence: 0.85,
            fallback_confidence: 0.75,
        }),
    },
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["#"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
