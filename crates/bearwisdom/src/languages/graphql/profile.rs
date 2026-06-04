use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const GRAPHQL_KIND_TABLE: KindTable = &[(
    EdgeKind::TypeRef,
    &[
        SymbolKind::Class,
        SymbolKind::Interface,
        SymbolKind::Enum,
        SymbolKind::Struct,
        SymbolKind::TypeAlias,
    ],
)];

pub const GRAPHQL_PROFILE: LanguageProfile = LanguageProfile {
    id: "graphql",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    members_can_be_external: false,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: true,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: &[],
    kind_compatible_table: GRAPHQL_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: Some(super::hooks::is_graphql_builtin),
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["#", "\"\"\""],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
