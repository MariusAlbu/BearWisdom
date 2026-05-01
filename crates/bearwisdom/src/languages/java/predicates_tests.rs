use super::predicates;

#[test]
fn test_mock_framework_methods_not_classified_as_java_builtin() {
    // JUnit / AssertJ / Mockito / Spring MockMvc method names used to be
    // classified as Java builtins. They are third-party libraries indexed
    // via Maven / Gradle externals; the Spring MockMvc DSL has its own
    // walker (`ecosystem/spring_stubs.rs`). The bare names collide with
    // common project-side method names (`given`, `when`, `then`, `verify`,
    // `perform`).
    for name in &[
        // Mockito BDDMockito
        "given", "when", "then", "verify", "willReturn",
        // AssertJ
        "assertThat", "isEqualTo", "isNotNull",
        // JUnit assertions
        "assertEquals", "assertTrue", "assertFalse",
        "assertNotNull", "assertThrows",
        // Spring MockMvc
        "perform", "andExpect",
    ] {
        assert!(
            !predicates::is_java_builtin(name),
            "{name:?} should not be classified as a java builtin",
        );
    }
}

#[test]
fn real_java_builtins_still_classified() {
    // Sanity: java.lang types / Object methods / Collection / Stream
    // methods still match.
    for name in &[
        // java.lang
        "System", "String", "Integer", "Object", "Class", "Math",
        "Exception", "Throwable", "RuntimeException",
        "IllegalArgumentException", "NullPointerException",
        // Object methods
        "toString", "equals", "hashCode",
        // String methods
        "length", "charAt", "substring", "indexOf",
        // Collection / Map / Set
        "add", "remove", "size", "stream", "forEach",
        "containsKey", "keySet", "values",
        // Stream
        "map", "filter", "reduce", "collect", "toList",
    ] {
        assert!(
            predicates::is_java_builtin(name),
            "{name:?} must remain a java builtin",
        );
    }
}
