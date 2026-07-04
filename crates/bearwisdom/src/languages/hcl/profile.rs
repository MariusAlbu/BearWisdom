// LanguageProfile for HCL (Terraform / Packer / etc.).

use crate::type_checker::profile::language_profile::{
    ChainQualification, DispatchAxis, LanguageProfile, SupertypeDiscovery, PERMISSIVE_KIND_TABLE,
};

pub const HCL_PROFILE: LanguageProfile = LanguageProfile {
    id: "hcl",
    qname_separator: ".",
    // `var.X` / `local.X` carry a sigil head the bare-name probes strip so the
    // reference binds to the same-file `X` Variable / `local` attribute.
    self_keywords: &["var", "local"],
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
    single_inner_wrappers: &[],
    container_deref_targets: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: &[],
    kind_compatible_table: PERMISSIVE_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    // Terraform meta-references (`each.value`, `count.index`, `self`, `path.*`,
    // `terraform.*`) are runtime-provided, not project symbols — decline before
    // the ladder so a same-named local can't be bound and external
    // classification brands them.
    builtin_skip: Some(super::keywords::is_terraform_meta_ref),
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    wildcard_builtins: &[],
    import_resolution: None,
    import_module_path: crate::type_checker::profile::language_profile::ImportModulePath::None,
    module_anchor: crate::type_checker::profile::language_profile::ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: crate::type_checker::profile::language_profile::RelativeMarker::None,
    external_by_import: None,
    name_normalization: crate::type_checker::profile::language_profile::NameNormalization::None,
    module_scope: crate::type_checker::profile::language_profile::ModuleScope::Off,
    wildcard_match: crate::type_checker::profile::language_profile::WildcardMatch::QnameUnder,
    ext_match: crate::type_checker::profile::language_profile::ExtMatch::PkgSegment,
    // A dotted target whose head names an in-file `provider` block
    // (`google.compute_instance` → the `provider "google"` class) binds the
    // head to that declaration. A `_`-bearing head is a resource TYPE
    // (`aws_instance.web`), not an alias, and is declined by the strategy.
    head_alias: crate::type_checker::profile::language_profile::HeadAliasBind::OnSameFile {
        require_kind: Some("class"),
    },
    file_scoped_imports: crate::type_checker::profile::language_profile::FileScopedImports::Off,
    alias_module_qname: false,
    module_prefix_rewrites:
        crate::type_checker::profile::language_profile::ModulePrefixRewrites::Off,
    workspace_packages: false,
    reexport_barrel_stems: &["index"],
    self_package_root: None,
    wildcard_workspace_scope: false,
    overload_pick_all: false,
    argument_dependent_lookup: false,
    associated_type_projection: false,
    blanket_impl_resolution: false,
    ambient_globals: crate::type_checker::profile::language_profile::AmbientGlobals::Off,
    self_receiver_discovery:
        crate::type_checker::profile::language_profile::SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    // Terraform `var`/`local`/resource names are module-flat — every `.tf`
    // in a directory shares one namespace, so a `var.X` ref binds cross-file.
    namespaceless_global_type_lookup:
        crate::type_checker::profile::language_profile::NamespaceScope::Global,
    explicit_member_import: false,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &["#", "//"],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
