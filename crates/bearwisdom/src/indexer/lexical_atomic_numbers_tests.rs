use super::*;
fn value(text: &str, negative: bool) -> LitValue {
    decode(
        text,
        negative,
        &crate::languages::typescript::flow::ATOMIC_TYPES,
    )
    .unwrap()
}

#[test]
fn numeric_radices_round_once_and_share_decimal_identity() {
    assert_eq!(
        value("0x20000000000001", false),
        value("9007199254740993", false)
    );
    assert_eq!(
        value("0x20000000000003", false),
        value("9007199254740995", false)
    );
    assert_eq!(
        value("0xfffffffffffffffffffffffff", false),
        value("1267650600228229401496703205375", false)
    );
    assert_eq!(value("1_000", false), value("1e3", false));
    assert_eq!(value("0", true), value("0", false));
    assert_eq!(value("0b1010", false), value("0o12", false));
    assert_ne!(value("1", true), value("1", false));
}

#[test]
fn big_integer_identity_retains_bits_beyond_u64_and_normalizes_zero() {
    let exact = value("0x10000000000000000000000000n", true);
    assert_eq!(exact, value("1267650600228229401496703205376n", true));
    assert_ne!(exact, value("1267650600228229401496703205377n", true));
    assert_eq!(value("0n", true), value("0n", false));
    assert_ne!(value("1n", false), value("1", false));
}
