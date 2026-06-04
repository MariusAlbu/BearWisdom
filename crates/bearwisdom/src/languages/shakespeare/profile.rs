use crate::type_checker::profile::language_profile::{
    DispatchAxis, LanguageProfile, SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};

const fn minimal(id: &'static str) -> LanguageProfile {
    LanguageProfile {
        id,
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
        constructor_patterns: &[],
        class_builder_specs: &[],
        decorator_syntax: None,
        doc_comment_kinds: &[],
        visibility_keywords: &[],
    }
}

pub const HAMLET_PROFILE: LanguageProfile = minimal("hamlet");
pub const CASSIUS_PROFILE: LanguageProfile = minimal("cassius");
pub const LUCIUS_PROFILE: LanguageProfile = minimal("lucius");
pub const JULIUS_PROFILE: LanguageProfile = minimal("julius");

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
