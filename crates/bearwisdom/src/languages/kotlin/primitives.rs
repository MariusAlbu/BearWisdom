// =============================================================================
// kotlin/primitives.rs — Kotlin primitive types
// =============================================================================

/// Primitive and built-in type names for Kotlin.
/// Kotlin has no keyword primitives — all types are objects. Core Kotlin
/// types (Int, String, List, Map, Pair, coroutine primitives) and JDK
/// types are now indexed as external symbols by the KotlinStdlib and
/// JdkSrc ecosystems. Only names that exceed the resolver's ambiguity
/// limit (Array=13, Result=14 kind-compatible candidates), dependency-
/// gated types (Android, Compose, coroutines, kotlinx.serialization),
/// test annotations, and generic type parameter conventions remain.
pub(crate) const PRIMITIVES: &[&str] = &[
    // High-ambiguity core types — kept as disambiguation short-circuits
    "Array", "Result",
    // Coroutines — not in kotlin-stdlib; from kotlinx-coroutines
    "Job", "Deferred", "CoroutineScope", "CoroutineContext",
    "Flow", "StateFlow", "SharedFlow", "MutableStateFlow", "MutableSharedFlow",
    "Channel", "SendChannel", "ReceiveChannel",
    "Mutex", "Semaphore",
    "GlobalScope", "MainScope", "CoroutineStart",
    "launch", "async", "runBlocking", "withContext", "delay",
    "Dispatchers",
    // Android / Compose common types (no Android SDK ecosystem active here)
    "Context", "Intent", "Bundle", "View", "Activity", "Fragment",
    "ViewModel", "LiveData", "MutableLiveData",
    "Modifier", "Color", "Dp",
    // Annotations (Kotlin/JVM + kotlinx.serialization)
    "Parcelable", "Parcelize",
    "Suppress", "OptIn", "JvmStatic", "JvmOverloads",
    "JvmField", "JvmName", "Throws", "Volatile", "Transient",
    // Test annotations (JUnit/Kotest — not in stdlib)
    "Test", "BeforeEach", "AfterEach", "BeforeAll", "AfterAll",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S",
];
