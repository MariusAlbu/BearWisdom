// =============================================================================
// indexer/primitives.rs — Language primitive + external type sets
//
// Delegates to the LanguagePlugin trait via the plugin registry.
// Each plugin owns its own primitives, externals, and framework globals.
// This module provides the combined set used by the resolution engine.
// =============================================================================

/// Return the slice of primitive/built-in type names for a language.
/// Dispatches through the plugin registry — no hardcoded match arms.
pub fn primitives_for_language(lang: &str) -> &'static [&'static str] {
    crate::languages::default_registry().get(lang).keywords()
}

/// Build a `HashSet<&'static str>` of ALL names that should be classified as
/// external for a given language. Combines two sources:
///
///   1. **Primitives** — language keyword types (from `LanguagePlugin::primitives()`)
///   2. **Query builtins** — keywords extracted from tree-sitter highlights.scm at build time
///
/// The third source that used to live here — `LanguagePlugin::externals()`
/// — was deleted in Phase 6. Always-external runtime globals (jest, DOM,
/// Input/OS in GDScript, ...) now come from indexed stdlib ecosystems
/// registered in `EcosystemRegistry`, so the resolver finds them as real
/// symbols rather than through a hardcoded list.
///
/// Dependency-gated framework globals are added separately by the resolution
/// engine via `LanguagePlugin::framework_globals()`.
pub fn primitives_set_for_language(lang: &str) -> std::collections::HashSet<&'static str> {
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
        // Note: stdlib types (Option, Result, Vec, etc.) are indexed as external
        // symbols by the RustStdlib ecosystem — no longer in the primitives list.
        assert!(p.contains(&"Fn"));
        assert!(p.contains(&"Self"));
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
        // Kotlin and Java have distinct primitive lists — Unit/Int are now
        // indexed by KotlinStdlib, Array/Result stay as ambiguity short-circuits.
        assert!(k.contains(&"Array"));
        assert!(k.contains(&"Flow"));
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
