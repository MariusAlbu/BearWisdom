use super::predicates;

#[test]
fn third_party_dsls_not_classified_as_groovy_builtin() {
    // SLF4J log methods, Spock test framework lifecycle/assertion keywords,
    // Gradle DSL top-level blocks, and MarkupBuilder HTML tag method names
    // were previously classified as Groovy builtins. They are third-party
    // DSL surfaces — Spock indexed via Maven, Gradle DSL via Gradle's own
    // synthetic stubs, MarkupBuilder is per-builder dynamic dispatch.
    for name in &[
        // SLF4J / log
        "info", "debug", "warn", "error", "trace",
        // Spock
        "setup", "given", "when", "then", "expect", "where", "cleanup",
        "Mock", "Stub", "Spy", "thrown", "notThrown",
        // Gradle DSL
        "apply", "plugins", "dependencies", "repositories",
        "configurations", "task", "sourceSets", "buildscript",
        "allprojects", "subprojects", "ext",
        // MarkupBuilder HTML tags (collide with arbitrary user method names)
        "tr", "th", "td", "table", "div", "span", "a",
        "h1", "h2", "h3", "ul", "ol", "li", "p",
        "img", "input", "br", "hr", "item", "mkp",
        // XML / JSON builders
        "make", "build",
    ] {
        assert!(
            !predicates::is_groovy_builtin(name),
            "{name:?} should not be classified as a groovy builtin",
        );
    }
}

#[test]
fn real_groovy_dgm_still_classified() {
    // Sanity: Groovy DefaultGroovyMethods (DGM) — the actual Groovy stdlib
    // surface — still match.
    for name in &[
        // DGM collection
        "each", "eachWithIndex", "collect", "find", "findAll",
        "any", "every", "inject", "groupBy", "flatten",
        // DGM Object
        "with", "tap", "asType", "metaClass", "println", "print",
        // DGM string
        "stripIndent", "stripMargin", "isInteger",
    ] {
        assert!(
            predicates::is_groovy_builtin(name),
            "{name:?} must remain a groovy builtin",
        );
    }
}
