use super::BICEP_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn bicep_profile_identity_and_shadow_mode() {
    assert_eq!(BICEP_PROFILE.id, "bicep");
}

#[test]
fn bicep_profile_declines_azure_resource_types() {
    // builtin_skip declines Azure resource type names (`Microsoft.Web/sites`)
    // before the ladder.
    assert!(BICEP_PROFILE.builtin_skip.is_some());
    let is_builtin = BICEP_PROFILE.builtin_skip.unwrap();
    assert!(is_builtin("Microsoft.Web/sites"));
    assert!(!is_builtin("myParameter"));
}

#[test]
fn bicep_profile_strips_sys_az_namespace_aliases() {
    // `sys`/`az` are namespace aliases over the bicep-runtime ambient symbols;
    // the engine strips them before the ambient-package lookup so `sys.concat`
    // resolves against the bare `concat` symbol.
    assert_eq!(BICEP_PROFILE.ambient_namespace_prefixes, &["sys", "az"]);
}

#[test]
fn bicep_declares_list_wildcard_builtin() {
    // The `az` `list*` regex overload is expressed as a single anchored
    // wildcard folding onto the vendored `list` family base.
    assert_eq!(BICEP_PROFILE.wildcard_builtins.len(), 1);
    let wb = &BICEP_PROFILE.wildcard_builtins[0];
    assert_eq!(wb.fold("listConnectionStrings"), Some("list"));
    assert_eq!(wb.fold("listener"), None);
    assert_eq!(wb.fold("list"), None);
}

#[test]
fn bicep_kind_table_gates_calls_to_callables() {
    // The kind table replaces PERMISSIVE — Calls accepts callables, not types.
    let table = BICEP_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Calls,
        SymbolKind::Function
    ));
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Calls,
        SymbolKind::Method
    ));
    assert!(!KindCompatibility::check(
        table,
        EdgeKind::Calls,
        SymbolKind::Class
    ));
}
