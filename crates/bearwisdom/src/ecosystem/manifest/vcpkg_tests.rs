use super::*;

#[test]
fn parses_string_dependency_list() {
    let json = r#"{ "name": "myapp", "dependencies": ["fmt", "boost-asio", "zlib"] }"#;
    let data = parse_vcpkg_json(json);
    assert!(data.dependencies.contains("fmt"));
    assert!(data.dependencies.contains("boost-asio"));
    assert!(data.dependencies.contains("zlib"));
}

#[test]
fn parses_object_form() {
    let json = r#"{
        "dependencies": [
            "fmt",
            { "name": "qt", "features": ["quick"] }
        ]
    }"#;
    let data = parse_vcpkg_json(json);
    assert!(data.dependencies.contains("fmt"));
    assert!(data.dependencies.contains("qt"));
}

#[test]
fn name_extraction() {
    let json = r#"{ "name": "myapp", "version-string": "1.0" }"#;
    assert_eq!(parse_vcpkg_name(json), Some("myapp".to_string()));
}

#[test]
fn empty_on_malformed_json() {
    let data = parse_vcpkg_json("not json");
    assert!(data.dependencies.is_empty());
}
