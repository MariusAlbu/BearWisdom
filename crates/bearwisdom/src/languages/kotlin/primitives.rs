// =============================================================================
// kotlin/primitives.rs — Kotlin primitive types
// =============================================================================

/// Primitive and built-in type names for Kotlin.
/// Kotlin has no keyword primitives — all types are objects.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Core types
    "Int", "Long", "Float", "Double", "Boolean", "Char", "Byte", "Short",
    "String", "Unit", "Any", "Nothing", "Number",
    "UInt", "ULong", "UByte", "UShort",
    // Exceptions
    "Throwable", "Exception", "RuntimeException", "Error",
    "IllegalArgumentException", "IllegalStateException",
    "NullPointerException", "IndexOutOfBoundsException",
    "UnsupportedOperationException", "ConcurrentModificationException",
    "NoSuchElementException", "ClassCastException",
    // Result / functional
    "Result", "Lazy", "Sequence", "Comparable",
    // Collections
    "Array", "List", "MutableList", "Map", "MutableMap", "Set", "MutableSet",
    "Pair", "Triple", "ArrayList", "HashMap", "HashSet", "LinkedHashMap",
    "LinkedHashSet", "ArrayDeque",
    "Collection", "MutableCollection", "Iterable", "MutableIterable",
    "Iterator", "MutableIterator", "ListIterator",
    // Coroutines
    "Job", "Deferred", "CoroutineScope", "CoroutineContext",
    "Flow", "StateFlow", "SharedFlow", "MutableStateFlow", "MutableSharedFlow",
    "Channel", "SendChannel", "ReceiveChannel",
    "Mutex", "Semaphore",
    // IO
    "InputStream", "OutputStream", "BufferedReader", "BufferedWriter",
    "ByteArray", "CharArray", "IntArray", "LongArray", "FloatArray", "DoubleArray",
    // Android / Compose common types
    "Context", "Intent", "Bundle", "View", "Activity", "Fragment",
    "ViewModel", "LiveData", "MutableLiveData",
    "Modifier", "Color", "Dp",
    // Annotations (Kotlin + JVM)
    "Serializable", "Parcelable", "Parcelize",
    "Suppress", "OptIn", "Deprecated", "JvmStatic", "JvmOverloads",
    "JvmField", "JvmName", "Throws", "Volatile", "Transient",
    "Test", "BeforeEach", "AfterEach", "BeforeAll", "AfterAll",
    // Coroutine builders/scopes
    "GlobalScope", "MainScope", "CoroutineStart",
    "launch", "async", "runBlocking", "withContext", "delay",
    "Dispatchers",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S",
];
