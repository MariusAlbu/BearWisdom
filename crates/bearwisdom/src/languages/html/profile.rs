// Minimal LanguageProfile for HTML. Markup language with no chains.
// Embedded scripts/styles route through their host language's profile.

use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, LanguageProfile, NameTransform, SelectorResolution,
    SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};
use crate::types::EdgeKind;

pub const HTML_PROFILE: LanguageProfile = LanguageProfile {
    id: "html",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: crate::type_checker::profile::language_profile::AncestorOrder::Bfs,
    members_can_be_external: false,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: false,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
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
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    package_by_directory: false,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::Off,
    file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
    alias_module_qname: false,
    module_prefix_rewrites: crate::type_checker::profile::language_profile::ModulePrefixRewrites::Off,
    workspace_packages: false,
    overload_pick_all: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::Off,
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    // A custom-element tag (`<user-card>` → `UserCard`) `Calls` ref binds to
    // the class registered under that tag by `customElements.define()`. The
    // raw target is tried first, then the kebab form; the map is keyed only on
    // real `define()` declarations, so a library tag with no project-side
    // define declines instead of binding to a coincidental same-named symbol.
    selector_resolution: Some(SelectorResolution {
        edge_kinds: &[EdgeKind::Calls],
        name_transforms: &[NameTransform::PascalToKebab],
    }),
    namespaceless_global_type_lookup: false,
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &[],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
