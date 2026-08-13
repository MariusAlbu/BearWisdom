use super::*;

#[test]
fn ecosystem_id_roundtrip() {
    let id = EcosystemId::new("maven");
    assert_eq!(id.as_str(), "maven");
    assert_eq!(format!("{id}"), "maven");
}
