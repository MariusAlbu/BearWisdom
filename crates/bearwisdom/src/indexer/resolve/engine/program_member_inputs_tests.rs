use super::*;
use crate::indexer::lexical::{
    globals::member_surface::{Kind, Signature},
    BindingId,
};
use crate::types::SourceSpan;

#[test]
fn lowering_preserves_missing_navigation_rows_and_source_binding_domains() {
    let member = Member {
        span: SourceSpan { start: 5, end: 31 },
        key_span: None,
        kind: Kind::Method,
        key: Key::Computed {
            expression: SourceSpan { start: 6, end: 17 },
            root: Root::Binding(BindingId(7)),
            selectors: vec![],
            usage: Default::default(),
        },
        modifiers: vec![],
        signature: Signature::default(),
        slot: Some(2),
    };
    let input = lower(&member, "api.d.ts", &SymbolIds::default());
    assert!(
        input.slot.is_none(),
        "absent row must not be reconstructed from a member name"
    );
    assert!(matches!(
        input.key,
        Key::Computed {
            root: Root::Binding(7),
            ..
        }
    ));
    let serialized = serde_json::to_value(&input).unwrap();
    let restored: Input = serde_json::from_value(serialized.clone()).unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), serialized);
}
