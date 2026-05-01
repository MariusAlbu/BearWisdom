// =============================================================================
// scala/keywords.rs — Scala primitive types
// =============================================================================

/// Primitive and built-in type names for Scala.
/// Scala core types (Int, String, List, Seq, Option, Either, Future,
/// Promise, exceptions, collections, etc.) are now indexed as external
/// symbols by the ScalaStdlib ecosystem via scala-library sources jar
/// in ~/.m2/repository. JDK types (Throwable, RuntimeException, etc.)
/// come from JdkSrc.
///
/// High-ambiguity core types stay as resolver short-circuits
/// (Option=16, Array=14, Map=11 kind-compatible external candidates).
/// NPM-like library names (cats/ZIO/Akka/Play/Scala.js) stay because
/// they're not part of scala-library.
pub(crate) const KEYWORDS: &[&str] = &[
    // High-ambiguity types kept as short-circuits
    "Option", "Array", "Map",
    // Operators (Scala methods called via special syntax)
    "->", ":=", "==", "!=", "&&", "||", "<", ">", "<=", ">=",
    "::", "++", "+:", ":+", "++:", ":::", "\\", "|", "&",
    "+=", "-=", "*=", "/=", "%=", "+", "-", "*", "/", "%",
    ">>", "<<", ">>>", "~", "^", "##",
    // Common method names (not types)
    "so", "into", "pp", "tap",
    // ScalaJS types (not in scala-library)
    "HTMLElement", "HTMLDivElement", "HTMLInputElement", "HTMLButtonElement",
    "document", "window", "console", "JSON",
    // Synthetic
    "_primitive",
    // Predef.assert — Scala stdlib assertion macro
    "assert",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S", "A", "B", "F",
    // From former builtin_type_names:
    "Int", "Long", "Double", "Float", "String", "Boolean", "Unit",
    "Any", "AnyRef", "AnyVal", "Nothing", "Null", "Char", "Byte", "Short",
    "Some", "None", "Left", "Right", "Nil",
    "List", "Vector", "Set", "Seq", "Either",
    "Try", "Success", "Failure", "Future", "Promise",
    "Tuple", "BigInt", "BigDecimal",
];
