use super::*;

#[test]
fn symbol_kind_roundtrip() {
    for kind in [
        SymbolKind::Class,
        SymbolKind::Struct,
        SymbolKind::Interface,
        SymbolKind::Enum,
        SymbolKind::EnumMember,
        SymbolKind::Method,
        SymbolKind::Constructor,
        SymbolKind::Property,
        SymbolKind::Field,
        SymbolKind::Namespace,
        SymbolKind::Event,
        SymbolKind::Delegate,
        SymbolKind::Function,
        SymbolKind::TypeAlias,
        SymbolKind::Variable,
        SymbolKind::Test,
    ] {
        let s = kind.as_str();
        let back = SymbolKind::from_str(s);
        assert_eq!(back, Some(kind), "round-trip failed for {kind:?}");
    }
}

#[test]
fn symbol_kind_display_matches_as_str() {
    for kind in [
        SymbolKind::Class,
        SymbolKind::EnumMember,
        SymbolKind::TypeAlias,
        SymbolKind::Test,
    ] {
        assert_eq!(kind.to_string(), kind.as_str());
    }
}

#[test]
fn edge_kind_roundtrip() {
    for kind in [
        EdgeKind::Calls,
        EdgeKind::Inherits,
        EdgeKind::Implements,
        EdgeKind::TypeRef,
        EdgeKind::Instantiates,
        EdgeKind::Imports,
        EdgeKind::HttpCall,
        EdgeKind::DbEntity,
        EdgeKind::LspResolved,
    ] {
        let s = kind.as_str();
        let back = EdgeKind::from_str(s);
        assert_eq!(back, Some(kind), "round-trip failed for {kind:?}");
    }
}

#[test]
fn edge_kind_display_matches_as_str() {
    for kind in [EdgeKind::HttpCall, EdgeKind::LspResolved, EdgeKind::DbEntity] {
        assert_eq!(kind.to_string(), kind.as_str());
    }
}

#[test]
fn visibility_roundtrip() {
    for v in [
        Visibility::Public,
        Visibility::Private,
        Visibility::Protected,
        Visibility::Internal,
    ] {
        let s = v.as_str();
        let back = Visibility::from_str(s);
        assert_eq!(back, Some(v), "round-trip failed for {v:?}");
    }
}

#[test]
fn visibility_display_matches_as_str() {
    for v in [Visibility::Public, Visibility::Protected] {
        assert_eq!(v.to_string(), v.as_str());
    }
}

#[test]
fn unknown_strings_return_none() {
    assert!(SymbolKind::from_str("Class").is_none()); // PascalCase rejected
    assert!(SymbolKind::from_str("").is_none());
    assert!(EdgeKind::from_str("http-call").is_none()); // wrong separator
    assert!(Visibility::from_str("Public").is_none()); // PascalCase rejected
}
