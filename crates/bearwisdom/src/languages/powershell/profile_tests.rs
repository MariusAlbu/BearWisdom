use super::POWERSHELL_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn powershell_profile_identity_and_shadow_mode() {
    assert_eq!(POWERSHELL_PROFILE.id, "powershell");
    assert_eq!(POWERSHELL_PROFILE.self_keywords, &["$this"]);
}

#[test]
fn powershell_kind_table_matches_former_predicate() {
    let t = POWERSHELL_PROFILE.kind_compatible_table;
    // Calls accepts the callable kinds plus class (cmdlet-style construction).
    for k in [
        SymbolKind::Method,
        SymbolKind::Function,
        SymbolKind::Constructor,
        SymbolKind::Test,
        SymbolKind::Class,
    ] {
        assert!(KindCompatibility::check(t, EdgeKind::Calls, k));
    }
    assert!(!KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Variable));
    // TypeRef accepts the type-and-value kinds the predicate allowed.
    for k in [
        SymbolKind::Class,
        SymbolKind::Interface,
        SymbolKind::Enum,
        SymbolKind::TypeAlias,
        SymbolKind::Function,
        SymbolKind::Variable,
    ] {
        assert!(KindCompatibility::check(t, EdgeKind::TypeRef, k));
    }
}
