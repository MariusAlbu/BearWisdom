use super::parse_generic_param_clause;

#[test]
fn names_only_when_unbounded() {
    let parsed = parse_generic_param_clause("K, V");
    assert_eq!(
        parsed,
        vec![("K".to_string(), None), ("V".to_string(), None)]
    );
}

#[test]
fn ts_extends_bound() {
    let parsed = parse_generic_param_clause("T extends Animal");
    assert_eq!(parsed, vec![("T".to_string(), Some("Animal".to_string()))]);
}

#[test]
fn colon_bound() {
    let parsed = parse_generic_param_clause("T: Animal");
    assert_eq!(parsed, vec![("T".to_string(), Some("Animal".to_string()))]);
}

#[test]
fn mixed_bounded_and_unbounded() {
    let parsed = parse_generic_param_clause("T extends Animal, U");
    assert_eq!(
        parsed,
        vec![
            ("T".to_string(), Some("Animal".to_string())),
            ("U".to_string(), None),
        ]
    );
}

#[test]
fn higher_kinded_marker_has_no_bound() {
    // Scala `F[_]` — the bracketed shape is not a bound.
    let parsed = parse_generic_param_clause("F[_]");
    assert_eq!(parsed, vec![("F".to_string(), None)]);
}

#[test]
fn rust_multibound_keeps_first() {
    let parsed = parse_generic_param_clause("T: Clone + Send");
    assert_eq!(parsed, vec![("T".to_string(), Some("Clone".to_string()))]);
}

#[test]
fn default_value_dropped_from_bound() {
    let parsed = parse_generic_param_clause("T extends Animal = Dog");
    assert_eq!(parsed, vec![("T".to_string(), Some("Animal".to_string()))]);
}

#[test]
fn scala_upper_bound_is_caught_via_colon() {
    // Scala `[T <: Animal]` — the `:` in `<:` already triggers the bound
    // branch, and the name split on `<` keeps the name clean.
    let parsed = parse_generic_param_clause("T <: Animal");
    assert_eq!(parsed, vec![("T".to_string(), Some("Animal".to_string()))]);
}

#[test]
fn generic_bound_preserved() {
    let parsed = parse_generic_param_clause("T extends Repository<User>");
    assert_eq!(
        parsed,
        vec![("T".to_string(), Some("Repository<User>".to_string()))]
    );
}
