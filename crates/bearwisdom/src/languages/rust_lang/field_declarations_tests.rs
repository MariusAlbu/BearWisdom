use super::*;

#[test]
fn raw_field_identifiers_are_decoded_once_without_changing_source_addresses() {
    let source = "pub struct Holder { pub r#type:Doc, pub r#item:Doc }";
    let result = crate::languages::rust_lang::extract::extract(source);
    let fields: Vec<_> = result
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Field)
        .collect();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "type");
    assert_eq!(fields[1].name, "item");
    assert_eq!(fields[0].qualified_name, "Holder.type");
    assert_eq!(fields[1].qualified_name, "Holder.item");
    assert_eq!(
        fields[0].start_col,
        source.find("pub r#type").unwrap() as u32
    );
    assert_eq!(fields[0].parent_index, fields[1].parent_index);
}
