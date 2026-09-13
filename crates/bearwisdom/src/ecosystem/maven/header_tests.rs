use super::scan_jvm_header;

/// The names a Kotlin header scan must surface for a file whose top-level
/// functions carry type parameters, modifiers and annotations — the shape of
/// a test assertion library.
#[test]
fn kotlin_top_level_functions_with_type_parameters_and_modifiers_are_indexed() {
    let src = "package kotlin.test\n\n\
        import kotlin.contracts.contract\n\n\
        /** Asserts equality. */\n\
        fun <@OnlyInputTypes T> assertEquals(expected: T, actual: T, message: String? = null) {\n\
        }\n\n\
        @JvmName(\"assertTrueLazy\")\n\
        inline fun assertTrue(message: String? = null, block: () -> Boolean) {\n\
            contract { callsInPlace(block) }\n\
        }\n\n\
        public fun fail(message: String? = null): Nothing = throw AssertionError(message)\n\n\
        internal expect fun <T> platformSpecific(value: T): T\n\n\
        annotation class Test\n\n\
        object Asserter\n";
    let mut names = scan_jvm_header(src, "kotlin");
    names.sort();
    names.dedup();
    for expected in [
        "assertEquals",
        "assertTrue",
        "fail",
        "platformSpecific",
        "Test",
        "Asserter",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "{expected} is a top-level declaration of the file; got {names:?}"
        );
    }
    assert!(
        !names.iter().any(|n| n == "T"),
        "a type parameter is not a declaration name; got {names:?}"
    );
}
