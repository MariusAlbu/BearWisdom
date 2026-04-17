// =============================================================================
// indexer/keywords.rs — Language keyword sets for the resolver
//
// Thin aggregator over `LanguagePlugin::keywords()` and the query-extracted
// highlights.scm builtins. The resolver uses the combined set to classify
// unresolvable references as "external" rather than "unresolved".
// =============================================================================

/// Return the slice of language-intrinsic keyword names declared by a
/// plugin. Dispatches through the plugin registry — no hardcoded match arms.
pub fn keywords_for_language(lang: &str) -> &'static [&'static str] {
    crate::languages::default_registry().get(lang).keywords()
}

/// Build a `HashSet<&'static str>` of ALL names that should be classified as
/// external for a given language. Combines two sources:
///
///   1. **Plugin keywords** — language keywords, operators, compiler intrinsics,
///      primitive type names, and syntax literals (from `LanguagePlugin::keywords()`)
///   2. **Query builtins** — identifiers marked as builtins in each grammar's
///      `highlights.scm`, extracted at build time.
///
/// Runtime globals and stdlib identifiers (jest, DOM, Input/OS in GDScript,
/// ...) come from indexed stdlib ecosystems registered in `EcosystemRegistry`,
/// so the resolver finds them as real symbols rather than through a hardcoded
/// list.
pub fn keywords_set_for_language(lang: &str) -> std::collections::HashSet<&'static str> {
    let plugin = crate::languages::default_registry().get(lang);
    let mut set: std::collections::HashSet<&'static str> =
        plugin.keywords().iter().copied().collect();
    // Merge in query-extracted builtins from tree-sitter .scm files.
    for name in super::query_builtins::query_builtins_for_language(lang) {
        set.insert(name);
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescript_keywords_include_string_and_number() {
        let p = keywords_for_language("typescript");
        assert!(p.contains(&"string"));
        assert!(p.contains(&"number"));
        assert!(p.contains(&"Promise"));
    }

    #[test]
    fn rust_keywords_include_i32_and_str() {
        let p = keywords_for_language("rust");
        assert!(p.contains(&"i32"));
        assert!(p.contains(&"str"));
        // Note: stdlib types (Option, Result, Vec, etc.) are indexed as external
        // symbols by the RustStdlib ecosystem — no longer in the keywords list.
        assert!(p.contains(&"Fn"));
        assert!(p.contains(&"Self"));
    }

    #[test]
    fn go_keywords_include_error() {
        let p = keywords_for_language("go");
        assert!(p.contains(&"error"));
        assert!(p.contains(&"string"));
    }

    #[test]
    fn unknown_language_returns_empty() {
        assert!(keywords_for_language("brainfuck").is_empty());
    }

    #[test]
    fn tsx_same_as_typescript() {
        assert_eq!(
            keywords_for_language("tsx"),
            keywords_for_language("typescript")
        );
    }

    #[test]
    fn kotlin_has_own_keywords() {
        let k = keywords_for_language("kotlin");
        let j = keywords_for_language("java");
        // Kotlin and Java have distinct keyword lists.
        assert!(k.contains(&"Array"));
        assert!(k.contains(&"Flow"));
        assert!(!std::ptr::eq(k, j));
    }

    #[test]
    fn fsharp_has_own_keywords() {
        let f = keywords_for_language("fsharp");
        let c = keywords_for_language("csharp");
        // F# uses "unit" and "obj", C# does not
        assert!(f.contains(&"unit"));
        assert!(f.contains(&"obj"));
        assert!(!std::ptr::eq(f, c));
    }
}
