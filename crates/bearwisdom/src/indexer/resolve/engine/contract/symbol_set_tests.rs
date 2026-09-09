use super::*;

#[test]
fn optional_symbol_candidates_borrow_the_original_declaration() {
    let symbol = super::super::super::testkit::sym(71, "read", "Doc.read", "method", "doc.rs");
    let set = SymbolSet::from(Some(&symbol));
    assert!(matches!(set, SymbolSet::Borrowed(_)));
    assert_eq!(set.len(), 1);
    assert!(std::ptr::eq(set.first().unwrap(), &symbol));
    assert!(SymbolSet::from(None).is_empty());
}
