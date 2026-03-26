use super::*;

#[test]
fn symbol_kind_roundtrip() {
    let kinds = [
        SymbolKind::Class,
        SymbolKind::Namespace,
        SymbolKind::Method,
        SymbolKind::Test,
        SymbolKind::TypeAlias,
    ];
    for k in &kinds {
        let s = k.as_str();
        let back = SymbolKind::from_str(s).unwrap_or_else(|| panic!("No from_str for {s}"));
        assert_eq!(*k, back);
    }
}

#[test]
fn edge_kind_roundtrip() {
    let kinds = [
        EdgeKind::Calls,
        EdgeKind::Inherits,
        EdgeKind::Implements,
        EdgeKind::TypeRef,
        EdgeKind::Instantiates,
        EdgeKind::HttpCall,
        EdgeKind::DbEntity,
        EdgeKind::LspResolved,
    ];
    for k in &kinds {
        let s = k.as_str();
        let back = EdgeKind::from_str(s).unwrap_or_else(|| panic!("No from_str for {s}"));
        assert_eq!(*k, back);
    }
}

#[test]
fn visibility_roundtrip() {
    for v in [Visibility::Public, Visibility::Private, Visibility::Protected, Visibility::Internal] {
        let s = v.as_str();
        let back = Visibility::from_str(s).unwrap();
        assert_eq!(v, back);
    }
}
