use super::*;

#[test]
fn normalise_interpolated_strips_query_and_params() {
    let bases = vec![];
    assert_eq!(
        normalise_interpolated_url("{ApiUrlBase}/items/{id}?foo=bar", &bases),
        "{*}/items/{*}"
    );
}

#[test]
fn normalise_interpolated_inlines_api_base() {
    let bases = vec![("".to_string(), "api/catalog".to_string())];
    assert_eq!(
        normalise_interpolated_url("{ApiUrlBase}/items", &bases),
        "api/catalog/items"
    );
}

#[test]
fn normalise_double_slash_cleaned() {
    let bases = vec![("".to_string(), "api/catalog".to_string())];
    assert_eq!(
        normalise_interpolated_url("{ApiUrlBase}//items", &bases),
        "api/catalog/items"
    );
}

#[test]
fn infer_method_finds_get() {
    let content = "var uri = buildUri();\nvar result = await _provider.GetAsync<Foo>(uri);";
    assert_eq!(infer_method_from_context(content, 0), "GET");
}

#[test]
fn infer_method_finds_post() {
    let content = "var data = new { };\nawait _provider.PostAsync(uri, data);";
    assert_eq!(infer_method_from_context(content, 0), "POST");
}

#[test]
fn infer_method_defaults_to_get() {
    let content = "var x = 42;\nvar y = 43;";
    assert_eq!(infer_method_from_context(content, 0), "GET");
}

#[test]
fn connect_runs_without_error_on_empty_db() {
    let db = crate::db::Database::open_in_memory().unwrap();
    let result = connect(db.conn(), std::path::Path::new("."));
    assert!(result.is_ok());
}
