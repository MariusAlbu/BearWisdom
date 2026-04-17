// =============================================================================
// groovy/predicates.rs — Groovy builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test" | "class"),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// Groovy control flow keywords that the grammar may parse as method_invocation.
pub(super) fn is_groovy_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "else" | "while" | "for" | "switch" | "case" | "do"
            | "try" | "catch" | "finally" | "throw" | "return"
            | "break" | "continue" | "assert"
    )
}

/// Groovy built-in methods, GDK additions, Spock lifecycle, and Gradle DSL
/// keywords that are never defined inside a project.
pub(super) fn is_groovy_builtin(name: &str) -> bool {
    matches!(
        name,
        // Groovy / GDK object methods
        "println"
            | "print"
            | "with"
            | "tap"
            | "collect"
            | "find"
            | "findAll"
            | "each"
            | "eachWithIndex"
            | "inject"
            | "any"
            | "every"
            | "sort"
            | "unique"
            | "groupBy"
            | "collectEntries"
            | "flatten"
            | "sum"
            | "min"
            | "max"
            | "count"
            | "size"
            | "first"
            | "last"
            | "head"
            | "tail"
            | "take"
            | "drop"
            | "toList"
            | "toSet"
            | "asType"
            | "getClass"
            | "toString"
            | "hashCode"
            | "equals"
            | "metaClass"
            | "getMetaClass"
            | "invokeMethod"
            | "getProperty"
            | "setProperty"
            // Spock framework lifecycle / assertion keywords
            | "setup"
            | "given"
            | "when"
            | "then"
            | "expect"
            | "where"
            | "cleanup"
            | "Mock"
            | "Stub"
            | "Spy"
            | "thrown"
            | "notThrown"
            // Gradle DSL top-level blocks / methods
            | "apply"
            | "plugins"
            | "dependencies"
            | "repositories"
            | "configurations"
            | "task"
            | "sourceSets"
            | "buildscript"
            | "allprojects"
            | "subprojects"
            | "ext"
    )
}
