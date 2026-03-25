//! Integration tests for bearwisdom-profile: project scanning and language detection.

use bearwisdom_profile::{scan, detect_language, find_language, find_language_by_extension, ScanOptions};
use bearwisdom_tests::TestProject;

// ── scan ────────────────────────────────────────────────────────────────

#[test]
fn scan_csharp_project() {
    let project = TestProject::csharp_service();
    let profile = scan(project.path(), ScanOptions::default());

    let langs: Vec<&str> = profile.languages.iter().map(|l| l.language_id.as_str()).collect();
    assert!(langs.contains(&"csharp"), "should detect C#, got: {langs:?}");
}

#[test]
fn scan_python_project() {
    let project = TestProject::python_app();
    let profile = scan(project.path(), ScanOptions::default());

    let langs: Vec<&str> = profile.languages.iter().map(|l| l.language_id.as_str()).collect();
    assert!(langs.contains(&"python"), "should detect Python, got: {langs:?}");
}

#[test]
fn scan_typescript_project() {
    let project = TestProject::typescript_app();
    let profile = scan(project.path(), ScanOptions::default());

    let langs: Vec<&str> = profile.languages.iter().map(|l| l.language_id.as_str()).collect();
    assert!(langs.contains(&"typescript"), "should detect TypeScript, got: {langs:?}");
}

#[test]
fn scan_multi_lang_project() {
    let project = TestProject::multi_lang();
    let profile = scan(project.path(), ScanOptions::default());

    assert!(
        profile.languages.len() >= 2,
        "should detect at least 2 languages, got {}",
        profile.languages.len(),
    );
}

#[test]
fn scan_empty_directory() {
    let dir = tempfile::TempDir::new().unwrap();
    let profile = scan(dir.path(), ScanOptions::default());

    assert!(profile.languages.is_empty(), "empty dir should detect no languages");
}

// ── detect_language ─────────────────────────────────────────────────────

#[test]
fn detect_language_known_extensions() {
    let cases = [
        ("foo.cs", "csharp"),
        ("bar.py", "python"),
        ("baz.ts", "typescript"),
        ("qux.js", "javascript"),
        ("main.rs", "rust"),
        ("App.java", "java"),
        ("main.go", "go"),
    ];

    for (filename, expected_id) in cases {
        let path = std::path::Path::new(filename);
        let detected = detect_language(path);
        assert!(
            detected.is_some(),
            "should detect language for {filename}",
        );
        assert_eq!(
            detected.unwrap().id, expected_id,
            "wrong language for {filename}",
        );
    }
}

#[test]
fn detect_language_unknown_extension() {
    let path = std::path::Path::new("mystery.xyz123");
    assert!(detect_language(path).is_none());
}

// ── registry lookups ────────────────────────────────────────────────────

#[test]
fn find_language_by_name() {
    assert!(find_language("csharp").is_some());
    assert!(find_language("python").is_some());
    assert!(find_language("nonexistent_lang").is_none());
}

#[test]
fn find_language_by_ext() {
    assert!(find_language_by_extension("cs").is_some());
    assert!(find_language_by_extension("py").is_some());
    assert!(find_language_by_extension("zzz").is_none());
}
