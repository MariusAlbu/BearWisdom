/// Groovy built-in methods, GDK collection extensions, Spock testing
/// constructs, and Gradle DSL keywords that are never defined inside a project.
pub(crate) const EXTERNALS: &[&str] = &[
    // -------------------------------------------------------------------------
    // Groovy / GDK object and collection methods
    // -------------------------------------------------------------------------
    "println", "print",
    "with", "tap",
    "collect", "find", "findAll",
    "each", "eachWithIndex",
    "inject",
    "any", "every",
    "sort", "unique",
    "groupBy", "collectEntries",
    "flatten",
    "sum", "min", "max", "count", "size",
    "first", "last", "head", "tail",
    "take", "drop",
    "toList", "toSet",
    "asType",
    "getClass", "toString", "hashCode", "equals",
    "metaClass", "getMetaClass",
    "invokeMethod", "getProperty", "setProperty",
    // -------------------------------------------------------------------------
    // Spock framework
    // -------------------------------------------------------------------------
    "setup", "given", "when", "then", "expect", "where", "cleanup",
    "Mock", "Stub", "Spy", "thrown", "notThrown",
    // -------------------------------------------------------------------------
    // Gradle DSL
    // -------------------------------------------------------------------------
    "apply", "plugins", "dependencies", "repositories",
    "configurations", "task", "sourceSets",
    "buildscript", "allprojects", "subprojects", "ext",
];
