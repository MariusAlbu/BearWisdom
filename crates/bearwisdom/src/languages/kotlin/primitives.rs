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
    // Android / Compose common types. Activity/Fragment/Intent/Bundle/
    // Parcelable live in android.jar sources and resolve via the
    // android-sdk ecosystem when $ANDROID_HOME/sources/android-<N>/ is
    // present. Context/View/Color stay as primitives — they're generic
    // enough that non-Android Kotlin projects also carry same-name types
    // and the ambiguity bite-back would regress those.
    "Context", "View",
    // Compose + androidx (live in Gradle caches, not Maven — ecosystem
    // doesn't scan them yet)
    "ViewModel", "LiveData", "MutableLiveData",
    "Modifier", "Color", "Dp",
    // kotlinx compiler plugin annotation (not in android.jar)
    "Parcelize",
    "Suppress", "OptIn", "JvmStatic", "JvmOverloads",
    "JvmField", "JvmName", "Throws", "Volatile", "Transient",
    // Test annotations (JUnit/Kotest — not in stdlib)
    "Test", "BeforeEach", "AfterEach", "BeforeAll", "AfterAll",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S",
];
