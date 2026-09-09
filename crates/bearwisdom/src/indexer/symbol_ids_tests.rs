use super::*;

#[test]
fn strict_identity_never_falls_back_to_qualified_name() {
    let mut ids = SymbolIds::default();
    ids.insert_key("a.ts".into(), "f.callback".into(), 99);
    assert_eq!(ids.row_id("a.ts", 0), None);
    ids.set_rows("a.ts".into(), vec![10, 0, 20]);
    assert_eq!(ids.row_id("a.ts", 0), Some(10));
    assert_eq!(ids.row_id("a.ts", 1), None);
    assert_eq!(ids.row_id("a.ts", 2), Some(20));
    assert_eq!(ids.row_id("a.ts", 3), None);
    assert_eq!(ids.row_id("b.ts", 0), None);
    ids.remap_ids(&HashMap::from([(20, 30)]));
    assert_eq!(ids.row_id("a.ts", 2), Some(30));
}
