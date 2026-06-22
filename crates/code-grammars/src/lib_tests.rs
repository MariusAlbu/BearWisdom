use super::*;

/// Every wired language id resolves to a grammar the parser accepts.
#[test]
fn all_wired_grammars_load_into_a_parser() {
    let langs = [
        "csharp", "typescript", "tsx", "javascript", "jsx", "python", "java", "go",
        "rust", "ruby", "php", "cpp", "c", "swift", "scala", "haskell", "elixir",
        "dart", "lua", "r", "html", "css", "scss", "json", "yaml", "xml", "bash",
        "sql", "kotlin", "markdown", "dockerfile", "graphql", "hcl", "proto", "nix",
        "zig", "cmake", "make", "gleam", "bicep", "odin", "starlark", "powershell",
        "groovy", "erlang", "fsharp", "gdscript", "pascal", "vbnet", "matlab",
        "clojure", "ocaml", "ada", "fortran",
    ];
    for lang in &langs {
        let language = get_language(lang).unwrap_or_else(|| panic!("get_language({lang}) returned None"));
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
    assert!(get_language("kotlin").is_some());
}
