// Accessor-property recovery and static-field emission over a synthetic
// metadata fixture: `get_X` yields Property `X`, a public static field is
// emitted as a Field row parented to the type, non-public / instance /
// special-name rows stay out.

use std::sync::{Arc, OnceLock};

use dotscope::metadata::signatures::SignatureField;
use dotscope::metadata::tables::Field;
use dotscope::metadata::token::Token;
use dotscope::metadata::typesystem::{CilFlavor, CilType};

use super::*;

#[test]
fn get_accessor_yields_property_name() {
    assert_eq!(
        accessor_property_name("get_InvariantCulture"),
        Some("InvariantCulture")
    );
    assert_eq!(accessor_property_name("set_Capacity"), Some("Capacity"));
    assert_eq!(accessor_property_name("get_"), None);
    assert_eq!(accessor_property_name("GetHashCode"), None);
    assert_eq!(accessor_property_name("Zero"), None);
}

#[test]
fn static_field_gate_admits_public_static_only() {
    // Public (0x6) + static (0x10).
    assert!(emittable_static_field(0x16, "Zero"));
    // Literal enum member: public + static + literal (0x20).
    assert!(emittable_static_field(0x36, "Monday"));
    // Instance field.
    assert!(!emittable_static_field(0x06, "length"));
    // Private static.
    assert!(!emittable_static_field(0x11, "cache"));
    // Special-name row.
    assert!(!emittable_static_field(0x216, "value__"));
    // Compiler-generated backing field.
    assert!(!emittable_static_field(0x16, "<Name>k__BackingField"));
}

fn synthetic_field(rid: u32, flags: u32, name: &str) -> Arc<Field> {
    Arc::new(Field {
        rid,
        token: Token::new(0x0400_0000 | rid),
        offset: 0,
        flags,
        name: name.to_string(),
        signature: SignatureField::default(),
        default: OnceLock::new(),
        rva: OnceLock::new(),
        layout: OnceLock::new(),
        marshal: OnceLock::new(),
        custom_attributes: Arc::new(boxcar::Vec::new()),
        declaring_type: OnceLock::new(),
    })
}

#[test]
fn static_field_is_emitted_parented_to_type() {
    let fields: Arc<boxcar::Vec<Arc<Field>>> = Arc::new(boxcar::Vec::new());
    fields.push(synthetic_field(1, 0x16, "Zero"));
    fields.push(synthetic_field(2, 0x06, "ticks"));
    fields.push(synthetic_field(3, 0x216, "value__"));
    let type_def = CilType::new(
        Token::new(0x0200_0001),
        "System".to_string(),
        "TimeSpan".to_string(),
        None,
        None,
        0x0000_0001,
        fields,
        Arc::new(boxcar::Vec::new()),
        Some(CilFlavor::Class),
    );
    let mut symbols = Vec::new();
    emit_static_fields(&type_def, "System.TimeSpan", 0, &mut symbols);
    assert_eq!(symbols.len(), 1, "only the public static field is emitted");
    let zero = &symbols[0];
    assert_eq!(zero.name, "Zero");
    assert_eq!(zero.qualified_name, "System.TimeSpan.Zero");
    assert_eq!(zero.kind, crate::types::SymbolKind::Field);
    assert_eq!(zero.parent_index, Some(0));
    assert_eq!(zero.scope_path.as_deref(), Some("System.TimeSpan"));
}
