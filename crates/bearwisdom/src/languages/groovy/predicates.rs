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

/// Groovy built-in methods, GDK/DGM additions, Spock lifecycle, and Gradle DSL
/// keywords that are never defined inside a project.
///
/// Covers `DefaultGroovyMethods` (DGM), `DefaultGroovyStaticMethods`,
/// `StringGroovyMethods`, `IOGroovyMethods`, and the Groovy MarkupBuilder DSL.
pub(super) fn is_groovy_builtin(name: &str) -> bool {
    matches!(
        name,
        // DefaultGroovyMethods — collection / Iterable
        "each"
            | "eachWithIndex"
            | "collect"
            | "collectEntries"
            | "collectMany"
            | "find"
            | "findAll"
            | "findResult"
            | "any"
            | "every"
            | "inject"
            | "sort"
            | "unique"
            | "groupBy"
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
            | "toSorted"
            | "addAll"
            | "join"
            | "push"
            | "pop"
            | "combinations"
            | "subsequences"
            | "permutations"
            | "transpose"
            | "intersect"
            | "disjoint"
            | "containsAll"
            | "withIndex"
            | "indexed"
            | "toUnique"
            // DefaultGroovyMethods — String / GDK StringGroovyMethods
            | "stripIndent"
            | "stripMargin"
            | "normalize"
            | "denormalize"
            | "readLines"
            | "splitEachLine"
            | "eachLine"
            | "eachMatch"
            | "findAll"
            | "replaceFirst"
            | "replaceAll"
            | "capitalize"
            | "uncapitalize"
            | "isInteger"
            | "isLong"
            | "isFloat"
            | "isDouble"
            | "isBigInteger"
            | "isBigDecimal"
            | "isNumber"
            | "toBigInteger"
            | "toBigDecimal"
            | "toInteger"
            | "toLong"
            | "toFloat"
            | "toDouble"
            | "reverse"
            | "format"
            // DefaultGroovyMethods — Object (mixed onto everything)
            | "with"
            | "tap"
            | "asType"
            | "asBoolean"
            | "getClass"
            | "toString"
            | "hashCode"
            | "equals"
            | "metaClass"
            | "getMetaClass"
            | "invokeMethod"
            | "getProperty"
            | "setProperty"
            | "println"
            | "print"
            | "printf"
            | "dump"
            | "inspect"
            | "is"
            | "use"
            | "identity"
            | "respondsTo"
            | "hasProperty"
            // DefaultGroovyMethods — Map
            | "subMap"
            | "withDefault"
            | "findAll"
            | "collectEntries"
            // DefaultGroovyStaticMethods
            | "sleep"
            // Groovy MarkupBuilder DSL (tag-name methods mixed on builder)
            | "tr"
            | "th"
            | "td"
            | "tr"
            | "table"
            | "tbody"
            | "thead"
            | "tfoot"
            | "caption"
            | "ul"
            | "ol"
            | "li"
            | "p"
            | "div"
            | "span"
            | "a"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "img"
            | "input"
            | "br"
            | "hr"
            | "item"
            | "mkp"
            // SLF4J / Groovy log methods (mixed via @Slf4j / @Log)
            | "info"
            | "debug"
            | "warn"
            | "error"
            | "trace"
            // Groovy XML / JSON builder DSL
            | "make"
            | "build"
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
