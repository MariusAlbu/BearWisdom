use super::*;

#[test]
fn empty_scope_matches_everything() {
    let scope = SearchScope::default();
    assert!(scope.matches_file("src/main.rs", "rust"));
    assert!(scope.matches_file("deep/nested/file.cs", "csharp"));
}

#[test]
fn language_filter() {
    let scope = SearchScope::default().with_language("rust");
    assert!(scope.matches_file("src/main.rs", "rust"));
    assert!(!scope.matches_file("src/app.ts", "typescript"));
}

#[test]
fn language_filter_case_insensitive() {
    let scope = SearchScope::default().with_language("CSharp");
    assert!(scope.matches_file("Foo.cs", "csharp"));
}

#[test]
fn directory_filter() {
    let scope = SearchScope::default().with_directory("src");
    assert!(scope.matches_file("src/main.rs", "rust"));
    assert!(!scope.matches_file("tests/test.rs", "rust"));
}

#[test]
fn include_glob() {
    let scope = SearchScope::default().with_include("*.rs");
    assert!(scope.matches_file("src/main.rs", "rust"));
    assert!(!scope.matches_file("src/app.ts", "typescript"));
}

#[test]
fn exclude_glob() {
    let scope = SearchScope::default().with_exclude("**test**");
    assert!(scope.matches_file("src/main.rs", "rust"));
    assert!(!scope.matches_file("src/tests/test_main.rs", "rust"));
}

#[test]
fn combined_filters() {
    let scope = SearchScope::default()
        .with_language("csharp")
        .with_directory("src")
        .with_exclude("**test**");

    assert!(scope.matches_file("src/Catalog/CatalogService.cs", "csharp"));
    assert!(!scope.matches_file("src/Catalog/CatalogService.cs", "typescript"));
    assert!(!scope.matches_file("tests/CatalogTests.cs", "csharp"));
    assert!(!scope.matches_file("src/tests/test.cs", "csharp"));
}

#[test]
fn detect_language() {
    assert_eq!(detect_language_from_path("foo.cs"), "csharp");
    assert_eq!(detect_language_from_path("bar.ts"), "typescript");
    assert_eq!(detect_language_from_path("baz.rs"), "rust");
    assert_eq!(detect_language_from_path("qux.py"), "python");
    assert_eq!(detect_language_from_path("no_ext"), "unknown");
}
