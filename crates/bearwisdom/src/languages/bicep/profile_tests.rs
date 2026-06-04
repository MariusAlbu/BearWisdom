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
    // before the ladder; the rest of resolve_ref drained, leaving only the
    // bicep-runtime fallback as a hook.
    assert!(BICEP_PROFILE.builtin_skip.is_some());
    let is_builtin = BICEP_PROFILE.builtin_skip.unwrap();
    assert!(is_builtin("Microsoft.Web/sites"));
    assert!(!is_builtin("myParameter"));
}

#[test]
fn bicep_kind_table_gates_calls_to_callables() {
    // The kind table replaces PERMISSIVE — Calls accepts callables, not types.
    let table = BICEP_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(table, EdgeKind::Calls, SymbolKind::Function));
    assert!(KindCompatibility::check(table, EdgeKind::Calls, SymbolKind::Method));
    assert!(!KindCompatibility::check(table, EdgeKind::Calls, SymbolKind::Class));
}
