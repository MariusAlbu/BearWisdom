// LanguageProfile for Fortran in shadow mode.

use crate::type_checker::core::types::PrimKind;
use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, KindTable, LanguageProfile, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

const FORTRAN_KIND_TABLE: KindTable = &[
    (
        EdgeKind::Calls,
        &[SymbolKind::Function, SymbolKind::Method],
    ),
    (EdgeKind::TypeRef, &[SymbolKind::Struct, SymbolKind::Module]),
];

const FORTRAN_PRIMITIVES: &[(&str, PrimKind)] = &[
    ("integer", PrimKind::Int),
    ("real", PrimKind::Float),
    ("double precision", PrimKind::Float),
    ("complex", PrimKind::Float),
    ("logical", PrimKind::Bool),
    ("character", PrimKind::Str),
];

pub const FORTRAN_PROFILE: LanguageProfile = LanguageProfile {
    id: "fortran",
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
    primitive_mapping: FORTRAN_PRIMITIVES,
    kind_compatible_table: FORTRAN_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::On(
        crate::type_checker::profile::language_profile::ModuleAnchorBind::MemberOfModuleType,
    ),
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::Spec(
        crate::type_checker::profile::language_profile::NormSpec {
            case_insensitive: true,
            strip_chars: &[],
            strip_prefixes: &[],
            strip_sigils: &[],
        },
    ),
    package_by_directory: false,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
    file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
    module_prefix_rewrites: crate::type_checker::profile::language_profile::ModulePrefixRewrites::Off,
    workspace_packages: false,
    overload_pick_all: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::Off,
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["!"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
