use std::collections::HashSet;

/// Runtime globals always external for Kotlin.
pub(crate) const EXTERNALS: &[&str] = &[
    // SLF4J logging (ubiquitous across JVM)
    "Logger", "LoggerFactory",
];

/// Dependency-gated framework globals for Kotlin.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // Spring framework (reuse Java's Spring constants)
    for dep in [
        "org.springframework.boot:spring-boot-starter-test",
        "org.springframework.boot",
        "spring-boot-starter-test",
        "org.springframework",
    ] {
        if deps.contains(dep) {
            globals.extend(SPRING_CORE);
            globals.extend(SPRING_TEST);
            break;
        }
    }

    // JOOQ SQL DSL
    for dep in ["org.jooq", "jooq", "org.jooq:jooq"] {
        if deps.contains(dep) {
            globals.extend(JOOQ_GLOBALS);
            break;
        }
    }

    // JUnit / Kotest
    for dep in ["junit", "org.junit.jupiter"] {
        if deps.contains(dep) {
            globals.extend(JUNIT_GLOBALS);
            break;
        }
    }
    if deps.contains("io.kotest") {
        globals.extend(KOTEST_GLOBALS);
    }

    // Konsist architecture testing
    for dep in ["com.lemonappdev.konsist", "konsist", "com.lemonappdev:konsist"] {
        if deps.contains(dep) {
            globals.extend(KONSIST_GLOBALS);
            break;
        }
    }

    // Android / Compose
    for dep in ["androidx.compose.ui", "compose", "androidx.compose"] {
        if deps.contains(dep) {
            globals.extend(COMPOSE_GLOBALS);
            break;
        }
    }
    for dep in ["androidx.fragment", "androidx.appcompat", "android"] {
        if deps.contains(dep) {
            globals.extend(ANDROID_GLOBALS);
            break;
        }
    }

    // Retrofit / OkHttp / networking
    for dep in ["com.squareup.retrofit2", "retrofit2", "com.squareup.okhttp3", "okhttp3"] {
        if deps.contains(dep) {
            globals.extend(NETWORK_GLOBALS);
            break;
        }
    }

    // Mockk
    for dep in ["io.mockk", "mockk"] {
        if deps.contains(dep) {
            globals.extend(MOCKK_GLOBALS);
            break;
        }
    }

    // Kotlin coroutines
    for dep in ["org.jetbrains.kotlinx:kotlinx-coroutines-core", "kotlinx-coroutines"] {
        if deps.contains(dep) {
            globals.extend(COROUTINE_GLOBALS);
            break;
        }
    }

    globals
}

const KONSIST_GLOBALS: &[&str] = &[
    "Konsist", "scopeFromProject", "scopeFromModule", "scopeFromPackage",
    "classes", "interfaces", "functions", "properties", "objects", "declarations",
    "assert", "assertTrue", "assertFalse", "assertEmpty", "assertNotEmpty",
    "hasModifier", "resideInPackage", "resideOutsidePackage",
    "hasAnnotation", "hasAnnotationOf", "hasReturnType",
    "hasNameStartingWith", "hasNameEndingWith", "hasNameContaining",
];

const COMPOSE_GLOBALS: &[&str] = &[
    // Compose UI test
    "waitForIdle", "onNodeWithText", "onNodeWithTag", "onNodeWithContentDescription",
    "onAllNodesWithText", "onAllNodesWithTag", "onRoot",
    "performClick", "performScrollTo", "performTextInput", "performTextClearance",
    "assertIsDisplayed", "assertIsNotDisplayed", "assertExists", "assertDoesNotExist",
    "assertTextEquals", "assertTextContains", "assertHasClickAction",
    // Compose runtime
    "remember", "mutableStateOf", "derivedStateOf", "produceState",
    "LaunchedEffect", "DisposableEffect", "SideEffect",
    "collectAsState", "collectAsStateWithLifecycle",
    // Compose Modifier
    "modifier", "Modifier",
];

const ANDROID_GLOBALS: &[&str] = &[
    "requireActivity", "requireContext", "requireView",
    "findNavController", "navigate", "popBackStack",
    "setContentView", "findViewById", "inflate",
    "startActivity", "finish", "getIntent",
    "getSharedPreferences", "getSystemService",
    "Toast", "makeText", "show",
    "Log", "d", "e", "w", "i", "v",
    "Timber",
    "runOnUiThread", "lifecycleScope", "viewModelScope",
    "launch", "async", "withContext",
];

const NETWORK_GLOBALS: &[&str] = &[
    "OkHttpClient", "Request", "Response", "Call", "Interceptor",
    "Retrofit", "GsonConverterFactory", "MoshiConverterFactory",
    "create", "build", "addInterceptor", "addConverterFactory",
    "baseUrl", "client",
];

const MOCKK_GLOBALS: &[&str] = &[
    "mockk", "every", "coEvery", "verify", "coVerify",
    "slot", "capture", "just", "runs", "returns", "answers",
    "confirmVerified", "clearMocks", "unmockkAll",
    "spyk", "relaxed",
];

const COROUTINE_GLOBALS: &[&str] = &[
    "launch", "async", "withContext", "delay",
    "runBlocking", "coroutineScope", "supervisorScope",
    "Dispatchers", "Main", "IO", "Default", "Unconfined",
    "flow", "collect", "emit", "map", "filter", "flatMapConcat",
    "stateIn", "shareIn", "launchIn",
    "runCatching", "getOrNull", "getOrElse", "getOrThrow",
    "also", "apply", "let", "run", "with", "takeIf", "takeUnless",
];

const SPRING_CORE: &[&str] = &[
    "RestController", "Controller", "Service", "Component", "Repository",
    "Configuration", "Bean", "Autowired", "Value", "Qualifier", "Primary",
    "Transactional", "Scheduled", "EventListener", "Async",
    "RequestMapping", "GetMapping", "PostMapping", "PutMapping", "DeleteMapping",
    "PatchMapping",
    "PathVariable", "RequestBody", "RequestParam", "RequestHeader",
    "ResponseEntity", "HttpStatus", "MediaType",
    "PageRequest", "Pageable", "Page", "Sort", "Specification",
];

const SPRING_TEST: &[&str] = &[
    "status", "content", "jsonPath", "xpath", "header", "cookie",
    "isOk", "isCreated", "isAccepted", "isNoContent",
    "isBadRequest", "isUnauthorized", "isForbidden", "isNotFound",
    "isConflict", "isInternalServerError",
    "contentType", "contentTypeCompatibleWith",
    "get", "post", "put", "patch", "delete",
    "accept", "param", "perform", "andExpect", "andReturn", "andDo",
    "MockBean", "SpyBean", "WebMvcTest", "SpringBootTest",
    "DataJpaTest", "AutoConfigureMockMvc",
    "assertThat", "isEqualTo", "isNotNull", "isNull", "isTrue", "isFalse",
    "hasSize", "contains", "containsExactly", "isEmpty", "isNotEmpty",
    "isInstanceOf", "extracting", "satisfies",
];

const JOOQ_GLOBALS: &[&str] = &[
    "select", "selectFrom", "selectCount", "selectDistinct",
    "insertInto", "update", "deleteFrom", "mergeInto",
    "from", "join", "leftJoin", "rightJoin", "crossJoin", "fullOuterJoin",
    "on", "and", "or", "not",
    "where", "having", "groupBy", "orderBy",
    "limit", "offset", "fetch", "fetchOne", "fetchAny", "fetchInto",
    "exists", "notExists", "in_", "notIn",
    "set", "values", "returning", "execute",
    "val", "field", "table", "name", "condition",
    "count", "sum", "avg", "min", "max",
    "coalesce", "nvl", "iif", "decode",
    "upper", "lower", "trim", "concat", "length", "substring",
    "cast", "coerce", "row", "asterisk",
    "dsl", "DSL",
];

const JUNIT_GLOBALS: &[&str] = &[
    "assertEquals", "assertThat", "assertTrue", "assertFalse",
    "assertNull", "assertNotNull", "verify", "when", "given", "mock",
];

const KOTEST_GLOBALS: &[&str] = &[
    "shouldBe", "shouldNotBe", "shouldThrow", "shouldNotThrow",
    "shouldBeNull", "shouldNotBeNull",
    "shouldBeEmpty", "shouldNotBeEmpty",
    "shouldContain", "shouldNotContain",
    "shouldHaveSize", "shouldBeGreaterThan", "shouldBeLessThan",
    "forAll", "forNone", "forExactly",
    "eventually", "continually",
];
