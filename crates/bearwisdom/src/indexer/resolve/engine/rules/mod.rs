// =============================================================================
// engine/rules — one resolution rule per file, in canonical ladder order
//
// `default_rules()` is the production rule set. The order is the same
// most-specific-evidence-first ladder the old strategy tower ran: a scope-chain
// bind beats a same-file bind beats a qualified-name bind beats an import-shape
// bind, and so on. Adding a case = adding a file here and one line in the list.
// =============================================================================

use super::LookupRule;

pub mod aliased_import;
pub mod alias_module_qname;
pub mod ambient_namespace_path;
pub mod ambient_prefix_strip;
pub mod ambient_scope;
pub mod builtin_skip;
pub mod chain_prefix;
pub mod component_import;
pub mod enclosing_member;
pub mod explicit_member_import;
pub mod external_by_import;
pub mod file_import;
pub mod file_scoped_import;
pub mod generic_param;
pub mod head_alias;
pub mod implicit_prelude;
pub mod imported_namespace;
pub mod import_path;
pub mod local_flow_head;
pub mod module_anchor;
pub mod module_anchor_terminal;
pub mod module_scope;
pub mod module_skip;
pub mod namespace_import;
pub mod namespaceless_global;
pub mod package_short_name;
pub mod qname_exact;
pub mod ranked_candidates;
pub mod reexport_chain;
pub mod reexport_following;
pub mod ref_module;
pub mod relative_module_wildcard;
pub mod same_file;
pub mod same_namespace;
pub mod scope_visible;
pub mod selector_map;
pub mod self_keyword;
pub mod wildcard_builtin_fold;
pub mod wildcard_import;
pub mod wildcard_workspace_package;
pub mod workspace_package;

use aliased_import::AliasedImportRule;
use alias_module_qname::AliasModuleQnameRule;
use ambient_namespace_path::AmbientNamespacePathRule;
use ambient_prefix_strip::AmbientPrefixStripRule;
use ambient_scope::AmbientScopeRule;
use builtin_skip::BuiltinSkipRule;
use chain_prefix::ChainPrefixRule;
use component_import::ComponentImportRule;
use enclosing_member::EnclosingMemberRule;
use explicit_member_import::ExplicitMemberImportRule;
use external_by_import::ExternalByImportRule;
use file_import::FileImportRule;
use file_scoped_import::FileScopedImportRule;
use generic_param::GenericParamRule;
use head_alias::HeadAliasRule;
use implicit_prelude::ImplicitPreludeRule;
use imported_namespace::ImportedNamespaceRule;
use import_path::ImportPathRule;
use local_flow_head::LocalFlowHeadRule;
use module_anchor::ModuleAnchorRule;
use module_anchor_terminal::ModuleAnchorTerminalRule;
use module_scope::ModuleScopeRule;
use module_skip::ModuleSkipRule;
use namespace_import::NamespaceImportRule;
use namespaceless_global::NamespacelessGlobalRule;
use package_short_name::PackageShortNameRule;
use qname_exact::QnameExactRule;
use ranked_candidates::RankedCandidatesRule;
use reexport_chain::ReexportChainRule;
use reexport_following::ReexportFollowingRule;
use ref_module::RefModuleRule;
use relative_module_wildcard::RelativeModuleWildcardRule;
use same_file::SameFileRule;
use same_namespace::SameNamespaceRule;
use scope_visible::ScopeVisibleRule;
use selector_map::SelectorMapRule;
use self_keyword::SelfKeywordRule;
use wildcard_builtin_fold::WildcardBuiltinFoldRule;
use wildcard_import::WildcardImportRule;
use wildcard_workspace_package::WildcardWorkspacePackageRule;
use workspace_package::WorkspacePackageRule;

/// The production rule set, in canonical ladder order. First rule that resolves
/// wins; a rule that stops the ladder leaves the ref honestly unresolved, and a
/// rule that drains it marks it a known non-project construct instead. The
/// order is the old strategy tower's `run_ladder` sequence: the module-decline
/// and builtin-drain guards, the most-specific evidence (import path /
/// workspace / selector / module anchor), then scope → local-flow-typed →
/// same-file → qualified → import-shape → ambient → wildcard → global rungs.
pub fn default_rules() -> Vec<Box<dyn LookupRule>> {
    vec![
        Box::new(ModuleSkipRule),
        Box::new(BuiltinSkipRule),
        Box::new(ImportPathRule),
        Box::new(WorkspacePackageRule),
        Box::new(SelectorMapRule),
        Box::new(ModuleAnchorRule),
        Box::new(ModuleAnchorTerminalRule),
        Box::new(ScopeVisibleRule),
        Box::new(LocalFlowHeadRule),
        Box::new(SameFileRule),
        Box::new(FileScopedImportRule),
        Box::new(SelfKeywordRule),
        Box::new(EnclosingMemberRule),
        Box::new(RefModuleRule),
        Box::new(QnameExactRule),
        Box::new(HeadAliasRule),
        Box::new(AliasModuleQnameRule),
        Box::new(ChainPrefixRule),
        Box::new(ReexportChainRule),
        Box::new(ExplicitMemberImportRule),
        Box::new(FileImportRule),
        Box::new(ComponentImportRule),
        Box::new(ReexportFollowingRule),
        Box::new(AliasedImportRule),
        Box::new(NamespaceImportRule),
        Box::new(PackageShortNameRule),
        Box::new(AmbientNamespacePathRule),
        Box::new(SameNamespaceRule),
        Box::new(ImportedNamespaceRule),
        Box::new(AmbientPrefixStripRule),
        Box::new(WildcardBuiltinFoldRule),
        Box::new(ExternalByImportRule),
        Box::new(ModuleScopeRule),
        Box::new(WildcardImportRule),
        Box::new(WildcardWorkspacePackageRule),
        Box::new(RelativeModuleWildcardRule),
        Box::new(ImplicitPreludeRule),
        Box::new(GenericParamRule),
        Box::new(AmbientScopeRule),
        Box::new(NamespacelessGlobalRule),
        Box::new(RankedCandidatesRule),
    ]
}
