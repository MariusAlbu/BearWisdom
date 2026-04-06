// =============================================================================
// indexer/primitives.rs — Language primitive type sets
//
// Delegates to per-language primitives files in languages/<lang>/primitives.rs.
// This module provides the language-ID routing (including aliases like tsx → TS)
// while each language owns its own primitive list.
// =============================================================================

/// Return the slice of primitive/built-in type names for the given language
/// identifier. Returns an empty slice for unrecognised languages.
pub fn primitives_for_language(lang: &str) -> &'static [&'static str] {
    match lang {
        "typescript" | "tsx" => crate::languages::typescript::primitives::PRIMITIVES,
        "javascript" | "jsx" => crate::languages::javascript::primitives::PRIMITIVES,
        "svelte" => crate::languages::typescript::primitives::PRIMITIVES,
        "astro" => crate::languages::typescript::primitives::PRIMITIVES,
        "vue" => crate::languages::typescript::primitives::PRIMITIVES,
        "angular" => crate::languages::typescript::primitives::PRIMITIVES,
        "java" => crate::languages::java::primitives::PRIMITIVES,
        "kotlin" => crate::languages::kotlin::primitives::PRIMITIVES,
        "scala" => crate::languages::scala::primitives::PRIMITIVES,
        "groovy" => crate::languages::groovy::primitives::PRIMITIVES,
        "csharp" => crate::languages::csharp::primitives::PRIMITIVES,
        "fsharp" => crate::languages::fsharp::primitives::PRIMITIVES,
        "vbnet" => crate::languages::vbnet::primitives::PRIMITIVES,
        "rust" => crate::languages::rust_lang::primitives::PRIMITIVES,
        "python" => crate::languages::python::primitives::PRIMITIVES,
        "go" => crate::languages::go::primitives::PRIMITIVES,
        "swift" => crate::languages::swift::primitives::PRIMITIVES,
        "dart" => crate::languages::dart::primitives::PRIMITIVES,
        "php" => crate::languages::php::primitives::PRIMITIVES,
        "ruby" => crate::languages::ruby::primitives::PRIMITIVES,
        "elixir" => crate::languages::elixir::primitives::PRIMITIVES,
        "sql" => crate::languages::sql::primitives::PRIMITIVES,
        _ => &[],
    }
}

/// Build a `HashSet<&'static str>` from the primitives for a given language.
/// Convenience wrapper for callers that need set membership checks.
pub fn primitives_set_for_language(lang: &str) -> std::collections::HashSet<&'static str> {
    primitives_for_language(lang).iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescript_primitives_include_string_and_number() {
        let p = primitives_for_language("typescript");
        assert!(p.contains(&"string"));
        assert!(p.contains(&"number"));
        assert!(p.contains(&"Promise"));
    }

    #[test]
    fn rust_primitives_include_i32_and_str() {
        let p = primitives_for_language("rust");
        assert!(p.contains(&"i32"));
        assert!(p.contains(&"str"));
        assert!(p.contains(&"Option"));
        assert!(p.contains(&"Result"));
    }

    #[test]
    fn go_primitives_include_error() {
        let p = primitives_for_language("go");
        assert!(p.contains(&"error"));
        assert!(p.contains(&"string"));
    }

    #[test]
    fn unknown_language_returns_empty() {
        assert!(primitives_for_language("brainfuck").is_empty());
    }

    #[test]
    fn tsx_same_as_typescript() {
        assert_eq!(
            primitives_for_language("tsx"),
            primitives_for_language("typescript")
        );
    }

    #[test]
    fn kotlin_has_own_primitives() {
        let k = primitives_for_language("kotlin");
        let j = primitives_for_language("java");
        // Kotlin uses "Unit" not "void", "Int" not "int"
        assert!(k.contains(&"Unit"));
        assert!(k.contains(&"Int"));
        assert!(!std::ptr::eq(k, j));
    }

    #[test]
    fn fsharp_has_own_primitives() {
        let f = primitives_for_language("fsharp");
        let c = primitives_for_language("csharp");
        // F# uses "unit" and "obj", C# does not
        assert!(f.contains(&"unit"));
        assert!(f.contains(&"obj"));
        assert!(!std::ptr::eq(f, c));
    }
}
