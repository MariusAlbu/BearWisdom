// =============================================================================
// java/primitives.rs — Java primitive types
// =============================================================================

/// Primitive and built-in type names for Java.
/// Includes keyword primitives, boxed wrappers, java.lang/java.util stdlib,
/// and generic type parameter names.
///
/// Dependency-gated types (JavaFX, ASM, etc.) live in `externals.rs`.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Keyword primitives
    "int", "long", "float", "double", "boolean", "char", "byte", "short", "void",
    // Boxed wrappers
    "Integer", "Long", "Float", "Double", "Boolean", "Character", "Byte", "Short",
    "String", "Object", "Void",
    // java.lang
    "Throwable", "Exception", "RuntimeException", "Error",
    "IllegalArgumentException", "IllegalStateException",
    "NullPointerException", "IndexOutOfBoundsException",
    "UnsupportedOperationException", "ClassCastException",
    "NoSuchElementException", "NumberFormatException",
    "Class", "ClassLoader", "Thread", "Runnable", "Comparable",
    "Iterable", "AutoCloseable", "Cloneable", "Enum",
    "System", "Math", "Runtime",
    "StringBuilder", "StringBuffer",
    "Override", "Deprecated", "SuppressWarnings", "FunctionalInterface",
    // java.util
    "List", "ArrayList", "LinkedList",
    "Map", "HashMap", "LinkedHashMap", "TreeMap", "ConcurrentHashMap",
    "Set", "HashSet", "LinkedHashSet", "TreeSet",
    "Collection", "Collections", "Arrays", "Iterator",
    "Optional", "OptionalInt", "OptionalLong", "OptionalDouble",
    "Stream", "Collectors",
    "Date", "Calendar", "UUID",
    "Queue", "Deque", "ArrayDeque", "PriorityQueue",
    // java.util.function
    "Function", "Consumer", "Supplier", "Predicate", "BiFunction", "BiConsumer",
    // java.io / java.nio
    "InputStream", "OutputStream", "Reader", "Writer",
    "File", "Path", "Paths", "Files",
    "IOException", "FileNotFoundException",
    "Serializable",
    // java.util.logging
    "Level",
    // Annotations
    "Nullable", "NonNull", "NotNull",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S",
];
