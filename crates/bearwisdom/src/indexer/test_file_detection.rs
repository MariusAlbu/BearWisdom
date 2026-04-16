// =============================================================================
// indexer/test_file_detection.rs — test-file path classifier
//
// Renamed from framework_globals.rs in Phase 8 cleanup — the hardcoded
// framework_globals() function that lived here predated the Ecosystem
// trait. After Phase 5 ships indexed stdlib + package ecosystems, this
// module is down to a single helper that flags path patterns typical of
// test suites so `dead_code` analysis can exclude them.
// =============================================================================

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
