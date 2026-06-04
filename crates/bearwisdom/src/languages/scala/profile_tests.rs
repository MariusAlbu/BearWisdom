use super::SCALA_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn scala_profile_identity() {
    assert_eq!(SCALA_PROFILE.id, "scala");
    assert_eq!(SCALA_PROFILE.qname_separator, ".");
    assert_eq!(SCALA_PROFILE.self_keywords, &["this", "super"]);
}

#[test]
fn scala_inherits_accepts_class_and_trait() {
    let t = SCALA_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Inherits, SymbolKind::Class));
    assert!(KindCompatibility::check(t, EdgeKind::Inherits, SymbolKind::Trait));
}

#[test]
fn scala_typeref_accepts_class_trait_enum_alias_module() {
    let t = SCALA_PROFILE.kind_compatible_table;
    for k in [
        SymbolKind::Class,
        SymbolKind::Trait,
        SymbolKind::Enum,
        SymbolKind::TypeAlias,
        SymbolKind::Module,
    ] {
        assert!(KindCompatibility::check(t, EdgeKind::TypeRef, k));
    }
}

#[test]
fn scala_async_wrappers_contain_future_io_task() {
    assert!(SCALA_PROFILE.async_wrappers.contains(&"Future"));
    assert!(SCALA_PROFILE.async_wrappers.contains(&"IO"));
    assert!(SCALA_PROFILE.async_wrappers.contains(&"Task"));
}
