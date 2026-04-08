// =============================================================================
// indexer/framework_globals.rs — Dependency-gated framework global detection
//
// Given a project's dependency names, returns the set of globally-injected
// names that frameworks provide (test globals, ORM DSLs, UI frameworks).
// These names never resolve to project symbols and should be classified
// as external rather than left unresolved.
//
// NOTE: This file is being migrated to per-language externals.rs files via
// the LanguagePlugin::framework_globals() trait method. New framework globals
// should be added to the language plugin, not here.
// =============================================================================

use std::collections::HashSet;

/// Given a set of dependency names (from the manifest), return the set of
/// global names injected by frameworks (test, ORM, UI).
///
/// This is the centralized dispatch — gradually being replaced by
/// `LanguagePlugin::framework_globals()` on each language plugin.
pub fn framework_globals(dependencies: &HashSet<String>) -> HashSet<String> {
    let mut globals = HashSet::new();

    // Svelte / SvelteKit — compiler-injected globals and virtual module types
    if dependencies.contains("svelte") || dependencies.contains("@sveltejs/kit") {
        globals.extend(SVELTE_GLOBALS.iter().map(|s| s.to_string()));
    }
    if dependencies.contains("@sveltejs/kit") {
        globals.extend(SVELTEKIT_GLOBALS.iter().map(|s| s.to_string()));
    }

    // i18n libraries (inject `$t`, `t`, `$i18n` etc.)
    for dep in [
        "svelte-i18n",
        "i18next",
        "next-i18next",
        "vue-i18n",
        "@ngx-translate/core",
    ] {
        if dependencies.contains(dep) {
            globals.extend(
                ["$t", "t", "$i18n", "i18n", "$locale", "$format"]
                    .iter()
                    .map(|s| s.to_string()),
            );
            break;
        }
    }

    // JS/TS ecosystem
    for dep in ["jest", "vitest", "@jest/globals", "mocha", "jasmine", "ava"] {
        if dependencies.contains(dep) {
            globals.extend(JS_TEST_GLOBALS.iter().map(|s| s.to_string()));
            break;
        }
    }
    for dep in ["playwright", "@playwright/test"] {
        if dependencies.contains(dep) {
            globals.extend(JS_TEST_GLOBALS.iter().map(|s| s.to_string()));
            globals.insert("page".to_string());
            globals.insert("browser".to_string());
            break;
        }
    }
    for dep in ["cypress"] {
        if dependencies.contains(dep) {
            globals.extend(JS_TEST_GLOBALS.iter().map(|s| s.to_string()));
            globals.insert("cy".to_string());
            break;
        }
    }

    // Python
    if dependencies.contains("pytest") {
        globals.extend(
            [
                "fixture", "mark", "parametrize", "raises", "approx", "monkeypatch",
                // pytest fixtures (auto-injected via conftest)
                "capsys", "capfd", "caplog", "tmp_path", "tmp_path_factory",
                "request", "pytestconfig", "cache", "doctest_namespace",
                "recwarn", "capfdbinary", "capsysbinary",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    }

    // JVM (Java / Kotlin / Scala / Groovy)
    for dep in ["junit", "org.junit.jupiter", "io.kotest", "org.scalatest"] {
        if dependencies.contains(dep) {
            globals.extend(
                [
                    "assertEquals",
                    "assertThat",
                    "assertTrue",
                    "assertFalse",
                    "assertNull",
                    "assertNotNull",
                    "verify",
                    "when",
                    "given",
                    "mock",
                ]
                .iter()
                .map(|s| s.to_string()),
            );
            break;
        }
    }
    // Kotest DSL (Kotlin)
    if dependencies.contains("io.kotest") {
        globals.extend(
            [
                "shouldBe", "shouldNotBe", "shouldThrow", "shouldNotThrow",
                "shouldBeNull", "shouldNotBeNull",
                "shouldBeEmpty", "shouldNotBeEmpty",
                "shouldContain", "shouldNotContain",
                "shouldHaveSize", "shouldBeGreaterThan", "shouldBeLessThan",
                "forAll", "forNone", "forExactly",
                "eventually", "continually",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    }
    // ScalaTest DSL
    if dependencies.contains("org.scalatest") {
        globals.extend(
            [
                "should", "must", "can", "in", "ignore",
                "FlatSpec", "WordSpec", "FunSuite", "FunSpec",
                "AnyFlatSpec", "AnyWordSpec", "AnyFunSuite", "AnyFunSpec",
                "Matchers", "BeforeAndAfter", "BeforeAndAfterAll",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    }

    // QUnit (JS testing)
    if dependencies.contains("qunit") {
        globals.extend(
            [
                "QUnit", "QUnit.test", "QUnit.module", "QUnit.skip",
                "QUnit.todo", "QUnit.only", "QUnit.start",
                "assert", "assert.expect", "assert.ok", "assert.notOk",
                "assert.equal", "assert.notEqual", "assert.strictEqual",
                "assert.notStrictEqual", "assert.deepEqual", "assert.notDeepEqual",
                "assert.propEqual", "assert.notPropEqual", "assert.propContains",
                "assert.true", "assert.false", "assert.throws", "assert.rejects",
                "assert.step", "assert.verifySteps", "assert.timeout",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    }

    // Benchmark.js
    if dependencies.contains("benchmark") {
        globals.extend(
            ["Benchmark", "Benchmark.Suite", "Benchmark.options"]
                .iter()
                .map(|s| s.to_string()),
        );
    }

    // Jasmine (JS testing — used by Bootstrap, Angular)
    for dep in ["jasmine", "jasmine-core", "karma-jasmine"] {
        if dependencies.contains(dep) {
            globals.extend(
                [
                    "spyOn", "jasmine", "jasmine.any", "jasmine.anything",
                    "jasmine.objectContaining", "jasmine.arrayContaining",
                    "jasmine.stringMatching", "jasmine.createSpy", "jasmine.createSpyObj",
                    "fixtureEl", "EventHandler",
                ]
                .iter()
                .map(|s| s.to_string()),
            );
            break;
        }
    }

    // .NET
    for dep in ["xunit", "nunit", "MSTest"] {
        if dependencies.contains(dep) {
            globals.extend(
                ["Assert", "Fact", "Theory", "TestMethod", "SetUp", "TearDown"]
                    .iter()
                    .map(|s| s.to_string()),
            );
            break;
        }
    }

    // -----------------------------------------------------------------------
    // Spring Test / MockMvc (Java & Kotlin)
    // -----------------------------------------------------------------------
    for dep in [
        "org.springframework.boot:spring-boot-starter-test",
        "org.springframework.boot",
        "spring-boot-starter-test",
        "org.springframework",
    ] {
        if dependencies.contains(dep) {
            globals.extend(SPRING_CORE_GLOBALS.iter().map(|s| s.to_string()));
            globals.extend(SPRING_TEST_GLOBALS.iter().map(|s| s.to_string()));
            break;
        }
    }

    // JOOQ (Kotlin/Java SQL DSL)
    for dep in ["org.jooq", "jooq", "org.jooq:jooq"] {
        if dependencies.contains(dep) {
            globals.extend(JOOQ_GLOBALS.iter().map(|s| s.to_string()));
            break;
        }
    }

    // Diesel (Rust ORM DSL)
    if dependencies.contains("diesel") {
        globals.extend(DIESEL_GLOBALS.iter().map(|s| s.to_string()));
    }

    // jQuery / Angular.js (legacy JS frameworks — injected globals)
    if dependencies.contains("jquery") || dependencies.contains("angular") {
        globals.extend(JQUERY_ANGULAR_GLOBALS.iter().map(|s| s.to_string()));
    }
    globals
}

const JS_TEST_GLOBALS: &[&str] = &[
    "expect", "it", "describe", "test", "beforeEach", "afterEach", "beforeAll", "afterAll",
    "vi", "jest", "mock", "spy", "fn", "assert", "should", "before", "after",
];

/// Svelte 5 runes and compiler-injected globals.
/// These are transformed by the Svelte compiler and never exist as importable symbols.
const SVELTE_GLOBALS: &[&str] = &[
    // Svelte 5 runes
    "$state", "$derived", "$effect", "$props", "$bindable", "$inspect",
    "$host", "$state.raw", "$derived.by", "$effect.pre", "$effect.root",
    // Svelte legacy reactive
    "$:", "$$props", "$$restProps", "$$slots",
    // Svelte store auto-subscription ($store syntax)
    // Note: specific store names are dynamic, but the `$` prefix pattern is Svelte convention.
];

/// SvelteKit virtual module types.
/// These come from `$app/*` and `./$types` virtual modules generated by SvelteKit.
const SVELTEKIT_GLOBALS: &[&str] = &[
    // ./$types virtual module
    "PageLoad", "PageData", "PageServerLoad", "PageServerData",
    "LayoutLoad", "LayoutData", "LayoutServerLoad", "LayoutServerData",
    "Actions", "ActionData", "RequestHandler",
    "EntryGenerator", "ParamMatcher",
    // $app/navigation
    "goto", "invalidate", "invalidateAll", "prefetch", "beforeNavigate",
    "afterNavigate", "onNavigate", "pushState", "replaceState",
    // $app/stores
    "page", "navigating", "updated",
    // $app/environment
    "browser", "building", "dev", "version",
    // $app/forms
    "enhance", "applyAction", "deserialize",
    // $app/paths
    "base", "assets", "resolveRoute",
    // $env/* virtual modules
    "env",
];

// ---------------------------------------------------------------------------
// Spring Test / MockMvc globals
// ---------------------------------------------------------------------------

const SPRING_CORE_GLOBALS: &[&str] = &[
    // Core annotations
    "RestController", "Controller", "Service", "Component", "Repository",
    "Configuration", "Bean", "Autowired", "Value", "Qualifier", "Primary",
    "Transactional", "Scheduled", "EventListener", "Async",
    // Request mapping
    "RequestMapping", "GetMapping", "PostMapping", "PutMapping", "DeleteMapping",
    "PatchMapping",
    "PathVariable", "RequestBody", "RequestParam", "RequestHeader",
    // Response types
    "ResponseEntity", "HttpStatus", "MediaType",
    // Data
    "PageRequest", "Pageable", "Page", "Sort", "Specification",
];

const SPRING_TEST_GLOBALS: &[&str] = &[
    // MockMvc result matchers
    "status", "content", "jsonPath", "xpath", "header", "cookie", "flash",
    "model", "view", "forwardedUrl", "redirectedUrl", "redirectedUrlPattern",
    // Status matchers
    "isOk", "isCreated", "isAccepted", "isNoContent",
    "isBadRequest", "isUnauthorized", "isForbidden", "isNotFound",
    "isConflict", "isInternalServerError",
    // Content matchers
    "contentType", "contentTypeCompatibleWith",
    // MockMvc request builders
    "get", "post", "put", "patch", "delete", "options", "head",
    "accept", "param", "params", "multipart",
    // MockMvc perform chain
    "perform", "andExpect", "andReturn", "andDo",
    // Spring test annotations
    "MockBean", "SpyBean", "WebMvcTest", "SpringBootTest",
    "DataJpaTest", "AutoConfigureMockMvc",
    // AssertJ (common in Spring projects)
    "assertThat", "isEqualTo", "isNotNull", "isNull", "isTrue", "isFalse",
    "hasSize", "contains", "containsExactly", "isEmpty", "isNotEmpty",
    "isInstanceOf", "extracting", "satisfies",
];

// ---------------------------------------------------------------------------
// JOOQ SQL DSL globals
// ---------------------------------------------------------------------------

const JOOQ_GLOBALS: &[&str] = &[
    // Query building
    "select", "selectFrom", "selectCount", "selectDistinct",
    "insertInto", "update", "deleteFrom", "mergeInto",
    "from", "join", "leftJoin", "rightJoin", "crossJoin", "fullOuterJoin",
    "on", "and", "or", "not",
    "where", "having", "groupBy", "orderBy",
    "limit", "offset", "fetch", "fetchOne", "fetchAny", "fetchInto",
    "exists", "notExists", "in_", "notIn",
    "set", "values", "returning", "execute",
    // DSL functions
    "val", "field", "table", "name", "condition",
    "count", "sum", "avg", "min", "max",
    "coalesce", "nvl", "iif", "decode",
    "upper", "lower", "trim", "concat", "length", "substring",
    "cast", "coerce",
    "row", "asterisk",
    "dsl", "DSL",
];

// ---------------------------------------------------------------------------
// Diesel ORM DSL globals (Rust)
// ---------------------------------------------------------------------------

const DIESEL_GLOBALS: &[&str] = &[
    // Query building
    "insert_into", "update", "delete", "replace_into",
    "select", "filter", "find", "first", "load", "get_result", "get_results",
    "execute", "returning", "on_conflict",
    "eq", "ne", "gt", "lt", "ge", "le",
    "and", "or", "not", "is_null", "is_not_null",
    "order", "order_by", "then_order_by", "desc", "asc",
    "limit", "offset", "group_by", "having",
    "inner_join", "left_join", "left_outer_join",
    "values", "set", "default_values",
    "as_select", "into_boxed", "distinct",
    // Diesel macros / DSL types
    "table", "joinable", "allow_tables_to_appear_in_same_query",
    "diesel", "with_lemmy_type", "auto_type", "derive_new",
    // Common chain methods
    "nullable", "is_contained_by", "contains",
];

// ---------------------------------------------------------------------------
// jQuery / Angular.js globals
// ---------------------------------------------------------------------------

const JQUERY_ANGULAR_GLOBALS: &[&str] = &[
    // jQuery
    "$", "jQuery",
    // Angular.js 1.x
    "angular", "angular.module", "angular.element", "angular.isObject",
    "angular.isArray", "angular.isString", "angular.isFunction",
    "angular.forEach", "angular.copy", "angular.extend",
    // Common Angular.js services
    "$scope", "$rootScope", "$http", "$state", "$stateParams",
    "$q", "$timeout", "$interval", "$window", "$document",
    "$compile", "$filter", "$location", "$log",
];

/// Check if a file path looks like a test file.
pub fn is_test_file(path: &str) -> bool {
    let p = path.replace('\\', "/");

    // Directory patterns
    if p.contains("/test/")
        || p.contains("/tests/")
        || p.contains("/__tests__/")
        || p.contains("/spec/")
        || p.contains("/specs/")
    {
        return true;
    }

    // File name patterns
    let name = p.rsplit('/').next().unwrap_or(&p);
    name.contains(".test.")
        || name.contains(".spec.")
        || name.contains("_test.")
        || name.starts_with("test_")
        || name.ends_with("Test.kt")
        || name.ends_with("Test.java")
        || name.ends_with("Tests.cs")
        || name.ends_with("Test.cs")
        || name.ends_with("Spec.scala")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jest_globals_injected() {
        let deps: HashSet<String> = ["jest"].iter().map(|s| s.to_string()).collect();
        let globals = framework_globals(&deps);
        assert!(globals.contains("expect"));
        assert!(globals.contains("describe"));
        assert!(globals.contains("it"));
        assert!(globals.contains("beforeEach"));
    }

    #[test]
    fn vitest_globals_injected() {
        let deps: HashSet<String> = ["vitest"].iter().map(|s| s.to_string()).collect();
        let globals = framework_globals(&deps);
        assert!(globals.contains("expect"));
        assert!(globals.contains("vi"));
    }

    #[test]
    fn playwright_adds_page_browser() {
        let deps: HashSet<String> = ["@playwright/test"].iter().map(|s| s.to_string()).collect();
        let globals = framework_globals(&deps);
        assert!(globals.contains("expect"));
        assert!(globals.contains("page"));
        assert!(globals.contains("browser"));
    }

    #[test]
    fn cypress_adds_cy() {
        let deps: HashSet<String> = ["cypress"].iter().map(|s| s.to_string()).collect();
        let globals = framework_globals(&deps);
        assert!(globals.contains("cy"));
    }

    #[test]
    fn no_test_framework_no_globals() {
        let deps: HashSet<String> = ["react", "axios"].iter().map(|s| s.to_string()).collect();
        let globals = framework_globals(&deps);
        assert!(globals.is_empty());
    }

    #[test]
    fn is_test_file_detects_patterns() {
        assert!(is_test_file("src/__tests__/foo.ts"));
        assert!(is_test_file("src/foo.test.ts"));
        assert!(is_test_file("src/foo.spec.js"));
        assert!(is_test_file("tests/test_foo.py"));
        assert!(is_test_file("UserServiceTest.java"));
        assert!(is_test_file("src/UserTests.cs"));
        assert!(!is_test_file("src/foo.ts"));
        assert!(!is_test_file("src/testing_utils.ts"));
    }
}
