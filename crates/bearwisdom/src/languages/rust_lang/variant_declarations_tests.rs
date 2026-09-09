use super::*;

#[test]
fn enum_payload_fields_retain_physical_parent_slots_and_type_addresses() {
    let source = "pub enum E<T> { Pair(T, crate::Doc), Named { r#type:T }, Empty }";
    let result = crate::languages::rust_lang::extract::extract(source);
    let fields: Vec<_> = result
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Field)
        .collect();
    assert_eq!(fields.len(), 3);
    assert_eq!(
        fields.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["0", "1", "type"]
    );
    assert_eq!(fields[0].start_col, source.find("T, crate").unwrap() as u32);
    assert_eq!(
        fields[1].start_col,
        source.find("crate::Doc").unwrap() as u32
    );
    for field in fields {
        let variant = &result.symbols[field.parent_index.unwrap()];
        assert_eq!(variant.kind, SymbolKind::EnumMember);
        assert_eq!(
            result.symbols[variant.parent_index.unwrap()].kind,
            SymbolKind::Enum
        );
    }
}
