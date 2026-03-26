use super::*;

#[test]
fn normalise_strips_api_prefix() {
    assert_eq!(normalise_route("/api/catalog/items"), "catalog/items");
}

#[test]
fn normalise_collapses_parameters() {
    assert_eq!(normalise_route("/api/items/{id:int}"), "items/{*}");
    assert_eq!(normalise_route("/api/items/{id}"),     "items/{*}");
}

#[test]
fn routes_match_identical() {
    assert!(routes_match("catalog/items", "catalog/items"));
}

#[test]
fn routes_match_with_parameter() {
    assert!(routes_match("catalog/items/{*}", "catalog/items/42"));
    assert!(routes_match("catalog/items/42", "catalog/items/{*}"));
}

#[test]
fn routes_no_match_different_segments() {
    assert!(!routes_match("catalog/items", "catalog/orders"));
    assert!(!routes_match("catalog/items/1", "catalog/items"));
}

#[test]
fn connect_runs_without_error_on_empty_db() {
    let db = Database::open_in_memory().unwrap();
    connect(&db).unwrap();
}
