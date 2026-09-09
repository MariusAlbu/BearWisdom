use super::*;

#[test]
fn profile_roles_are_distinct_serializable_id_keys() {
    assert_ne!(Kind::Mutable, Kind::Readonly);
    for kind in [Kind::Mutable, Kind::Readonly] {
        assert_eq!(
            serde_json::from_str::<Kind>(&serde_json::to_string(&kind).unwrap()).unwrap(),
            kind
        );
    }
}
