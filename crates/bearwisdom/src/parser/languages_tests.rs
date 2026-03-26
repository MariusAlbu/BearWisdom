use super::*;

/// Every language with a non-None get_language must load without panicking.
#[test]
fn all_supported_grammars_load() {
    let langs = [
        "csharp", "typescript", "tsx", "javascript", "jsx",
        "python", "java", "go", "rust", "ruby", "php", "cpp", "c",
        "swift", "scala", "haskell", "elixir", "dart", "lua", "r",
        "html", "css", "scss", "json", "yaml", "xml", "bash", "sql",
        "kotlin", "markdown", "dockerfile",
    ];
    for lang in &langs {
        let result = get_language(lang);
        assert!(result.is_some(), "get_language({lang}) returned None");
        // Verify the parser actually accepts it (grammar compiled correctly).
        let language = result.unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&language)
            .unwrap_or_else(|e| panic!("Parser::set_language failed for {lang}: {e}"));
    }
}

#[test]
fn unknown_language_returns_none() {
    assert!(get_language("cobol").is_none());
    assert!(get_language("").is_none());
    assert!(get_language("kotlin").is_some()); // now using tree-sitter-kotlin-ng
}

#[test]
fn has_extractor_only_for_known_languages() {
    assert!(has_extractor("csharp"));
    assert!(has_extractor("typescript"));
    assert!(has_extractor("tsx"));
    assert!(has_extractor("rust"));
    assert!(has_extractor("python"));
    assert!(has_extractor("go"));
    assert!(has_extractor("java"));
}
