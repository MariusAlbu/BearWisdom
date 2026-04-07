// =============================================================================
// scala/primitives.rs — Scala primitive types
// =============================================================================

/// Primitive and built-in type names for Scala.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Value types
    "Int", "Long", "Float", "Double", "Boolean", "Char", "Byte", "Short",
    "String", "Unit", "Any", "AnyRef", "AnyVal", "Nothing", "Null",
    // Option/Either variants
    "Some", "None", "Left", "Right", "Nil",
    // Collections and containers
    "List", "Vector", "Map", "Set", "Seq", "Array", "Option", "Either",
    "Try", "Success", "Failure", "Future", "Promise",
    "Tuple", "BigInt", "BigDecimal",
    "IndexedSeq", "LinearSeq", "Iterable", "Iterator", "Buffer",
    "ListBuffer", "ArrayBuffer", "Queue", "Stack", "TreeMap", "TreeSet",
    "SortedMap", "SortedSet", "LazyList", "Stream",
    "Range", "NumericRange",
    "HashMap", "HashSet", "LinkedHashMap", "LinkedHashSet",
    "ListMap", "TrieMap",
    "Tuple1", "Tuple2", "Tuple3", "Tuple4", "Tuple5",
    // Exceptions
    "Exception", "Throwable", "RuntimeException", "Error",
    "IllegalArgumentException", "IllegalStateException",
    "NoSuchElementException", "UnsupportedOperationException",
    "IndexOutOfBoundsException", "NullPointerException",
    "ClassCastException", "ArithmeticException",
    "MatchError", "NotImplementedError",
    // Concurrent / async
    "ExecutionContext", "Duration", "FiniteDuration",
    "Await", "Awaitable",
    // Functional
    "PartialFunction", "Function0", "Function1", "Function2",
    "Product", "Serializable",
    "Ordering", "Numeric", "Integral", "Fractional",
    // Type classes (cats/zio ecosystem)
    "IO", "Task", "ZIO", "UIO", "URIO",
    "Monad", "Functor", "Applicative", "Traverse",
    "Show", "Eq", "Order", "Semigroup", "Monoid",
    // Operators (these are method calls in Scala)
    "->", ":=", "==", "!=", "&&", "||", "<", ">", "<=", ">=",
    "::", "++", "+:", ":+", "++:", ":::", "\\", "|", "&",
    "+=", "-=", "*=", "/=", "%=", "+", "-", "*", "/", "%",
    ">>", "<<", ">>>", "~", "^", "##",
    // Common method names
    "so", "into", "pp", "tap",
    // ScalaJS types
    "HTMLElement", "HTMLDivElement", "HTMLInputElement", "HTMLButtonElement",
    "document", "window", "console", "JSON",
    // Synthetic
    "_primitive",
    // Play/Akka/http4s
    "Redirect", "Ok", "BadRequest", "NotFound", "InternalServerError",
    "Action", "Results", "Request", "Response", "Cookie",
    "Props", "ActorRef", "ActorSystem",
    // assertEquals (testing)
    "assertEquals", "assertNotEquals", "assertTrue", "assertFalse",
    "assertThrows", "assert",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S", "A", "B", "F",
];
