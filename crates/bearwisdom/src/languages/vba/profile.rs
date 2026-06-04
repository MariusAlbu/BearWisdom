use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, LanguageProfile, NameNormalization, NormSpec,
    SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};

pub const VBA_PROFILE: LanguageProfile = LanguageProfile {
    id: "vba",
    qname_separator: ".",
    self_keywords: &["Me"],
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
    // VBA identifiers are case-insensitive — a reference written in any casing
    // binds to a same-name candidate in the bare-name strategies.
    name_normalization: NameNormalization::Spec(NormSpec {
        case_insensitive: true,
        strip_chars: &[],
        strip_prefixes: &[],
        strip_sigils: &[],
    }),
    package_by_directory: false,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["'"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
