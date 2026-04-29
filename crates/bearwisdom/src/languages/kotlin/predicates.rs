// =============================================================================
// kotlin/predicates.rs — Kotlin builtin and helper predicates
// =============================================================================

use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::types::EdgeKind;

// ---------------------------------------------------------------------------
// Compose test DSL — hardcoded list of well-known functions
// ---------------------------------------------------------------------------

/// Common Jetpack Compose test DSL functions from `androidx.compose.ui.test`.
///
/// These are injected as implicit builtins in Android Compose test files rather
/// than indexing the full test jars, which is significantly cheaper and covers
/// the vast majority of real-world Compose UI test code.
const COMPOSE_TEST_DSL: &[&str] = &[
    // ComposeContentTestRule / ComposeTestRule
    "setContent",
    "waitForIdle",
    "waitUntil",
    "waitUntilAtLeastOneExists",
    "waitUntilDoesNotExist",
    "waitUntilExactlyOneExists",
    "waitUntilNodeCount",
    "runOnUiThread",
    "runOnIdle",
    "registerIdlingResource",
    "unregisterIdlingResource",
    // SemanticsNodeInteractionsProvider
    "onNode",
    "onAllNodes",
    "onNodeWithText",
    "onNodeWithTag",
    "onNodeWithContentDescription",
    "onAllNodesWithText",
    "onAllNodesWithTag",
    "onAllNodesWithContentDescription",
    "onRoot",
    // SemanticsMatcher factories
    "hasText",
    "hasContentDescription",
    "hasTestTag",
    "isDisplayed",
    "isEnabled",
    "isNotEnabled",
    "isFocused",
    "isNotFocused",
    "isSelected",
    "isNotSelected",
    "isToggleable",
    "isClickable",
    "isScrollable",
    "isHeading",
    "hasClickAction",
    "hasScrollAction",
    "hasSetTextAction",
    "hasAnyDescendant",
    "hasAnyAncestor",
    "hasAnySibling",
    "hasParent",
    "hasNoClickAction",
    // SemanticsNodeInteraction actions / assertions
    "assertExists",
    "assertDoesNotExist",
    "assertIsDisplayed",
    "assertIsNotDisplayed",
    "assertIsEnabled",
    "assertIsNotEnabled",
    "assertIsSelected",
    "assertIsNotSelected",
    "assertIsToggleable",
    "assertIsFocused",
    "assertIsNotFocused",
    "assertHasClickAction",
    "assertHasNoClickAction",
    "assertTextEquals",
    "assertTextContains",
    "assertContentDescriptionEquals",
    "assertContentDescriptionContains",
    "assertCountEquals",
    "performClick",
    "performTextInput",
    "performTextClearance",
    "performTextReplacement",
    "performImeAction",
    "performScrollTo",
    "performScrollToIndex",
    "performScrollToKey",
    "performGesture",
    "performTouchInput",
    "performMouseInput",
    "performSemanticsAction",
    "fetchSemanticsNode",
    "printToLog",
    "printToString",
    // Rule builders
    "createComposeRule",
    "createAndroidComposeRule",
    "createEmptyComposeRule",
];

/// Returns `true` if `name` is a well-known Jetpack Compose test DSL function
/// or assertion. These are considered builtins to avoid false unresolved refs
/// in Compose UI test files.
pub(super) fn is_compose_test_dsl(name: &str) -> bool {
    COMPOSE_TEST_DSL.contains(&name)
}

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// Always-external Kotlin/JVM namespace roots.
const ALWAYS_EXTERNAL: &[&str] = &[
    "kotlin",
    "kotlinx",
    "java",
    "javax",
    "jakarta",
    "android",
    "androidx",
    "org.junit",
    "org.assertj",
    "io.mockk",
    "org.springframework",
    "com.fasterxml",
    "io.ktor",
];

/// Check whether a Kotlin namespace or import path is external.
pub(super) fn is_external_kotlin_namespace(
    ns: &str,
    project_ctx: Option<&ProjectContext>,
) -> bool {
    for prefix in ALWAYS_EXTERNAL {
        if ns == *prefix || ns.starts_with(&format!("{prefix}.")) {
            return true;
        }
    }

    if let Some(ctx) = project_ctx {
        return is_manifest_jvm_external(ctx, ns);
    }

    false
}

/// Check whether a Kotlin/JVM namespace is external using Maven/Gradle manifests directly.
pub(super) fn is_manifest_jvm_external(ctx: &ProjectContext, ns: &str) -> bool {
    let root = ns.split('.').next().unwrap_or(ns);
    if matches!(root, "java" | "javax" | "jakarta" | "sun" | "org") {
        return true;
    }
    for kind in [ManifestKind::Maven, ManifestKind::Gradle] {
        if let Some(m) = ctx.manifest(kind) {
            if m.dependencies.contains(ns) {
                return true;
            }
            for dep in &m.dependencies {
                if ns.starts_with(dep.as_str()) {
                    return true;
                }
            }
        }
    }
    false
}

/// Check whether a fully-qualified target looks external.
pub(super) fn effective_target_is_external(
    target: &str,
    project_ctx: Option<&ProjectContext>,
) -> bool {
    if !target.contains('.') {
        return false;
    }
    is_external_kotlin_namespace(target, project_ctx)
}

/// Kotlin stdlib builtins always in scope without import.
/// Also covers Jetpack Compose test DSL which is injected via test rules.
pub(super) fn is_kotlin_builtin(name: &str) -> bool {
    if is_compose_test_dsl(name) {
        return true;
    }
    let root = name.split('.').next().unwrap_or(name);
    matches!(
        root,
        // stdlib scope functions
        "with"
            | "apply"
            | "run"
            | "let"
            | "also"
            | "takeIf"
            | "takeUnless"
            | "repeat"
            | "lazy"
            // result / exception handling
            | "runCatching"
            | "getOrElse"
            | "getOrDefault"
            | "getOrNull"
            | "getOrThrow"
            | "onSuccess"
            | "onFailure"
            | "recover"
            | "recoverCatching"
            // collection builders
            | "listOf"
            | "setOf"
            | "mapOf"
            | "mutableListOf"
            | "mutableSetOf"
            | "mutableMapOf"
            | "arrayOf"
            | "arrayOfNulls"
            | "emptyList"
            | "emptySet"
            | "emptyMap"
            | "emptyArray"
            | "buildString"
            | "buildList"
            | "buildMap"
            | "buildSet"
            | "sortedMapOf"
            | "sortedSetOf"
            | "linkedMapOf"
            | "linkedSetOf"
            | "hashMapOf"
            | "hashSetOf"
            | "sequenceOf"
            | "generateSequence"
            | "sequence"
            // collection extension stubs that appear as calls without receiver
            | "any"
            | "all"
            | "none"
            | "filter"
            | "filterNot"
            | "filterIsInstance"
            | "map"
            | "mapNotNull"
            | "flatMap"
            | "flatten"
            | "forEach"
            | "forEachIndexed"
            | "find"
            | "findLast"
            | "first"
            | "firstOrNull"
            | "last"
            | "lastOrNull"
            | "single"
            | "singleOrNull"
            | "count"
            | "sumOf"
            | "average"
            | "reduce"
            | "associate"
            | "associateBy"
            | "associateWith"
            | "groupBy"
            | "partition"
            | "unzip"
            | "take"
            | "drop"
            | "distinct"
            | "distinctBy"
            | "sorted"
            | "sortedBy"
            | "sortedDescending"
            | "reversed"
            | "toList"
            | "toSet"
            | "toMap"
            | "toMutableList"
            | "toMutableSet"
            | "joinToString"
            | "contains"
            | "containsAll"
            | "indexOf"
            | "indexOfFirst"
            | "indexOfLast"
            | "isEmpty"
            | "isNotEmpty"
            | "isNullOrEmpty"
            | "orEmpty"
            // preconditions / errors
            | "require"
            | "requireNotNull"
            | "check"
            | "checkNotNull"
            | "error"
            | "TODO"
            // math / comparisons
            | "maxOf"
            | "minOf"
            | "abs"
            | "coerceIn"
            | "coerceAtLeast"
            | "coerceAtMost"
            | "compareBy"
            | "compareByDescending"
            | "compareValues"
            | "compareValuesBy"
            | "naturalOrder"
            | "reverseOrder"
            // numeric conversions
            | "toByte"
            | "toShort"
            | "toInt"
            | "toLong"
            | "toFloat"
            | "toDouble"
            | "toChar"
            | "toBoolean"
            | "toString"
            // I/O
            | "println"
            | "print"
            | "readLine"
            | "readText"
            // coroutine helpers (kotlinx.coroutines — always imported in Android)
            | "launch"
            | "async"
            | "withContext"
            | "coroutineScope"
            | "supervisorScope"
            | "delay"
            | "flow"
            | "emit"
            | "collect"
            | "collectLatest"
            | "stateIn"
            | "shareIn"
            | "combine"
            | "flowOn"
            | "channelFlow"
            | "mapLatest"
            | "flatMapLatest"
            // identity / iteration
            | "it"
            // pseudo-keywords used as refs
            | "this"
            | "super"
            // built-in types always in scope (kotlin.*)
            | "String"
            | "Int"
            | "Long"
            | "Double"
            | "Float"
            | "Boolean"
            | "Byte"
            | "Short"
            | "Char"
            | "Unit"
            | "Nothing"
            | "Any"
            | "List"
            | "Map"
            | "Set"
            | "Array"
            | "Pair"
            | "Triple"
            | "Sequence"
            | "Result"
            | "Comparable"
            | "Iterable"
            | "Iterator"
            | "Collection"
            | "MutableList"
            | "MutableSet"
            | "MutableMap"
            | "MutableCollection"
            | "MutableIterable"
            | "HashMap"
            | "HashSet"
            | "LinkedHashMap"
            | "LinkedHashSet"
            | "ArrayList"
            | "Number"
            | "Enum"
            | "Throwable"
            | "Exception"
            | "RuntimeException"
            | "IllegalArgumentException"
            | "IllegalStateException"
            | "IndexOutOfBoundsException"
            | "NullPointerException"
            | "UnsupportedOperationException"
            | "NoSuchElementException"
            | "ArithmeticException"
            | "ClassCastException"
            | "StackOverflowError"
            // Kotlin stdlib serialization
            | "Serializable"
            // Kotlin reflection
            | "KClass"
            | "KFunction"
            | "KProperty"
            | "KType"
            // Annotation stubs always in scope
            | "Deprecated"
            | "Suppress"
            | "JvmStatic"
            | "JvmField"
            | "JvmOverloads"
            | "JvmName"
            | "Transient"
            | "Volatile"
            | "Synchronized"
            | "Throws"
            | "JvmSuppressWildcards"
            | "OptIn"
            | "RequiresOptIn"
            // Kotlin stdlib annotations
            | "InternalAPI"
            | "PublishedApi"
            | "ExperimentalApi"
            | "OptionalExpectation"
            | "DslMarker"
            | "BuilderInference"
            | "ContextDsl"
            | "JvmInline"
            | "JvmDefault"
            | "JvmDefaultWithCompatibility"
            | "Repeatable"
            | "MustBeDocumented"
            | "Retention"
            | "Target"
            // Kotlin stdlib types
            | "CharSequence"
            | "CharArray"
            | "ByteArray"
            | "ShortArray"
            | "IntArray"
            | "LongArray"
            | "FloatArray"
            | "DoubleArray"
            | "BooleanArray"
            | "UByte"
            | "UShort"
            | "UInt"
            | "ULong"
            | "UByteArray"
            | "UShortArray"
            | "UIntArray"
            | "ULongArray"
            | "IntRange"
            | "LongRange"
            | "CharRange"
            | "ClosedRange"
            | "Function0"
            | "Function1"
            | "Function2"
            | "Function3"
            | "Function4"
            | "KParameter"
            // kotlin.test framework
            | "assertTrue"
            | "assertFalse"
            | "assertNull"
            | "assertNotNull"
            | "assertEquals"
            | "assertNotEquals"
            | "assertSame"
            | "assertNotSame"
            | "assertContains"
            | "assertContentEquals"
            | "assertFails"
            | "assertFailsWith"
            | "assertIs"
            | "assertIsNot"
            | "assertNotEmpty"
            | "fail"
            // kotlinx.serialization
            | "KSerializer"
            | "SerialDescriptor"
            | "Serializer"
            | "Encoder"
            | "Decoder"
            | "CompositeEncoder"
            | "CompositeDecoder"
            | "PolymorphicSerializer"
            | "ContextualSerializer"
            | "SerialName"
            | "SerialKind"
            | "SerializationException"
            | "SerializationStrategy"
            | "DeserializationStrategy"
            // kotlinx.coroutines additions
            | "CompletableDeferred"
            | "Deferred"
            | "Job"
            | "SupervisorJob"
            | "CoroutineScope"
            | "CoroutineContext"
            | "CoroutineDispatcher"
            | "Dispatchers"
            | "Channel"
            | "BroadcastChannel"
            | "MutableSharedFlow"
            | "MutableStateFlow"
            | "StateFlow"
            | "SharedFlow"
            | "Flow"
            | "FlowCollector"
            | "ReceiveChannel"
            | "SendChannel"
            | "actor"
            | "produce"
            | "select"
            | "yield"
            // Kotlin/JS interop
            | "JsAny"
            | "JsNumber"
            | "JsString"
            | "JsBoolean"
            | "JsArray"
            | "JsName"
            | "JsExport"
            | "JsModule"
            | "JsNonModule"
            // Kotlin/Native cinterop
            | "CPointer"
            | "CValuesRef"
            | "CValue"
            | "CFunction"
            | "CStruct"
            | "ByteVar"
            | "ShortVar"
            | "IntVar"
            | "LongVar"
            | "FloatVar"
            | "DoubleVar"
            | "BooleanVar"
            | "UByteVar"
            | "UShortVar"
            | "UIntVar"
            | "ULongVar"
            | "CPointerVar"
            | "alloc"
            | "allocArray"
            | "memScoped"
            // Kotlin stdlib bare-name calls
            | "byteArrayOf"
            | "shortArrayOf"
            | "intArrayOf"
            | "longArrayOf"
            | "floatArrayOf"
            | "doubleArrayOf"
            | "booleanArrayOf"
            | "charArrayOf"
            | "ubyteArrayOf"
            | "ushortArrayOf"
            | "uintArrayOf"
            | "ulongArrayOf"
            | "synchronized"
            | "tailrec"
            // More kotlin.test annotations
            | "AfterTest"
            | "BeforeTest"
            | "Test"
            | "Ignore"
            | "BeforeClass"
            | "AfterClass"
            // More kotlinx.coroutines
            | "CompletableJob"
            | "CoroutineName"
            | "CancellableContinuation"
            | "Continuation"
            | "ContinuationInterceptor"
            | "EmptyCoroutineContext"
            | "EagerInitialization"
            | "ExperimentalCoroutinesApi"
            // More Kotlin/Native cinterop
            | "COpaquePointer"
            | "COpaquePointerVar"
            | "UnsafeBufferOperations"
            | "Pinned"
            | "usePinned"
            | "refTo"
            | "interpretCPointer"
            | "interpretNullablePointer"
            | "toCValues"
            | "cstr"
            | "wcstr"
            | "Char16"
            | "Char32"
            // Kotlin/Native Apple/Foundation interop names
            | "NSString"
            | "NSData"
            | "NSArray"
            | "NSDictionary"
            | "NSNumber"
            | "NSDate"
            | "NSURL"
            | "NSError"
            | "NSObject"
            | "NSCharacterSet"
            | "NSMutableArray"
            | "NSMutableDictionary"
            | "NSMutableString"
            | "NSCoder"
            | "NSNotification"
            // More Kotlin stdlib bare-name calls (string / number methods)
            | "trimIndent"
            | "trimMargin"
            | "decodeToString"
            | "encodeToByteArray"
            | "lowercase"
            | "uppercase"
            | "lowercaseChar"
            | "uppercaseChar"
            | "capitalize"
            | "padStart"
            | "padEnd"
            | "removePrefix"
            | "removeSuffix"
            | "removeSurrounding"
            | "splitToSequence"
            | "lines"
            | "asSequence"
            | "asIterable"
            | "asList"
            | "windowed"
            | "chunked"
            | "zipWithNext"
            | "fold"
            | "foldRight"
            | "scan"
            | "runningFold"
            | "runningReduce"
            // Annotations from common Ktor API surface
            | "ExperimentalKtorApi"
            | "ExperimentalSerializationApi"
            // Servlet API (Java/Kotlin web stacks)
            | "HttpServletRequest"
            | "HttpServletResponse"
            | "ServletRequest"
            | "ServletResponse"
            | "FilterChain"
            | "Filter"
            // Netty
            | "ChannelHandlerContext"
            | "ChannelHandler"
            | "ChannelInboundHandler"
            | "ChannelOutboundHandler"
    )
}
