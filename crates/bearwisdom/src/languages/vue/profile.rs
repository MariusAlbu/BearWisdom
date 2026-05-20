use crate::type_checker::profile::language_profile::{
    DispatchAxis, LanguageProfile, SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};

pub const VUE_PROFILE: LanguageProfile = LanguageProfile {
    id: "vue",
    qname_separator: ".",
    self_keywords: &["this"],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: false,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: &[],
    kind_compatible_table: PERMISSIVE_KIND_TABLE,
    engine_primary: false,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["<!--"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
