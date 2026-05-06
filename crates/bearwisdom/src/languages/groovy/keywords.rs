// =============================================================================
// groovy/keywords.rs — Groovy primitives + DGM/GDK Object-mixin methods
//
// String/Object/List/Map/BigDecimal/BigInteger come from JdkSrc +
// GroovyStdlib. The DGM (DefaultGroovyMethods) / GDK methods listed
// here are mixed onto every object at runtime — they have no
// declaration anywhere in user source and the receiver type is
// rarely inferrable for bare `obj.each {}` / `obj.collect {}` calls,
// so they're classified as primitives via the engine's keywords()
// set rather than chain-walking to a method symbol.
// =============================================================================

pub(crate) const KEYWORDS: &[&str] = &[
    // Keyword primitives
    "void", "boolean", "byte", "char", "short", "int", "long", "float", "double",
    // Groovy-specific keyword
    "def",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S",
    // Type names retained for parser-noise filtering
    "String", "Object", "List", "Map", "GString", "BigDecimal", "BigInteger",
    // DGM — collection / Iterable mixins
    "each", "eachWithIndex", "collect", "collectEntries", "collectMany",
    "find", "findAll", "findResult", "any", "every", "inject",
    "sort", "unique", "groupBy", "flatten", "sum", "min", "max", "count", "size",
    "first", "last", "head", "tail", "take", "drop",
    "toList", "toSet", "toSorted", "addAll", "join",
    "push", "pop", "combinations", "subsequences", "permutations",
    "transpose", "intersect", "disjoint", "containsAll",
    "withIndex", "indexed", "toUnique",
    // DGM — String / GDK StringGroovyMethods
    "stripIndent", "stripMargin", "normalize", "denormalize",
    "readLines", "splitEachLine", "eachLine", "eachMatch",
    "replaceFirst", "replaceAll", "capitalize", "uncapitalize",
    "isInteger", "isLong", "isFloat", "isDouble", "isBigInteger",
    "isBigDecimal", "isNumber",
    "toBigInteger", "toBigDecimal", "toInteger", "toLong", "toFloat", "toDouble",
    "reverse", "format",
    // DGM — Object (mixed onto everything)
    "with", "tap", "asType", "asBoolean",
    "getClass", "toString", "hashCode", "equals",
    "metaClass", "getMetaClass", "invokeMethod",
    "getProperty", "setProperty",
    "println", "print", "printf", "dump", "inspect",
    "is", "use", "identity", "respondsTo", "hasProperty",
    // DGM — Map
    "subMap", "withDefault",
    // DefaultGroovyStaticMethods
    "sleep",
];
