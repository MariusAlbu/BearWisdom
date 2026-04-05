// =============================================================================
// indexer/test_frameworks.rs — Test framework global injection detection
//
// Given a project's dependency names, returns the set of globally-injected
// names that test frameworks provide (e.g., `expect`, `describe`, `it` from
// Jest/Vitest). These names never resolve to project symbols and should be
// classified as external rather than left unresolved.
// =============================================================================

use std::collections::HashSet;

/// Given a set of dependency names (from the manifest), return the set of
/// global names injected by test frameworks.
pub fn test_framework_globals(dependencies: &HashSet<String>) -> HashSet<String> {
    let mut globals = HashSet::new();

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
            ["fixture", "mark", "parametrize", "raises", "approx", "monkeypatch"]
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

    globals
}

const JS_TEST_GLOBALS: &[&str] = &[
    "expect", "it", "describe", "test", "beforeEach", "afterEach", "beforeAll", "afterAll",
    "vi", "jest", "mock", "spy", "fn", "assert", "should", "before", "after",
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
        let globals = test_framework_globals(&deps);
        assert!(globals.contains("expect"));
        assert!(globals.contains("describe"));
        assert!(globals.contains("it"));
        assert!(globals.contains("beforeEach"));
    }

    #[test]
    fn vitest_globals_injected() {
        let deps: HashSet<String> = ["vitest"].iter().map(|s| s.to_string()).collect();
        let globals = test_framework_globals(&deps);
        assert!(globals.contains("expect"));
        assert!(globals.contains("vi"));
    }

    #[test]
    fn playwright_adds_page_browser() {
        let deps: HashSet<String> = ["@playwright/test"].iter().map(|s| s.to_string()).collect();
        let globals = test_framework_globals(&deps);
        assert!(globals.contains("expect"));
        assert!(globals.contains("page"));
        assert!(globals.contains("browser"));
    }

    #[test]
    fn cypress_adds_cy() {
        let deps: HashSet<String> = ["cypress"].iter().map(|s| s.to_string()).collect();
        let globals = test_framework_globals(&deps);
        assert!(globals.contains("cy"));
    }

    #[test]
    fn no_test_framework_no_globals() {
        let deps: HashSet<String> = ["react", "axios"].iter().map(|s| s.to_string()).collect();
        let globals = test_framework_globals(&deps);
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
