use super::LUA_PROFILE;
use crate::type_checker::profile::language_profile::{KindCompatibility, SupertypeDiscovery};

#[test]
fn lua_profile_identity() {
    assert_eq!(LUA_PROFILE.id, "lua");
    assert_eq!(LUA_PROFILE.self_keywords, &["self"]);
}

#[test]
fn lua_supertype_discovery_is_structural() {
    assert_eq!(
        LUA_PROFILE.supertype_discovery,
        SupertypeDiscovery::Structural
    );
}

#[test]
fn lua_calls_accepts_function_method_variable() {
    use crate::types::{EdgeKind, SymbolKind};
    let t = LUA_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Function
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Method
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Variable
    ));
}
