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

    globals
}

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
