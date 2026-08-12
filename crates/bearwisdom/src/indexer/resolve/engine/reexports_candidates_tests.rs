use super::*;

#[test]
fn relative_base_joins_and_collapses_dot_segments() {
    // `./sibling` from a barrel resolves to the barrel's directory.
    assert_eq!(
        relative_base("packages/q/src/index.ts", "./queryClient").as_deref(),
        Some("packages/q/src/queryClient")
    );
    // `..` from a nested test file climbs to the parent directory's stem.
    assert_eq!(
        relative_base("packages/q/src/__tests__/a.test.tsx", "..").as_deref(),
        Some("packages/q/src")
    );
    // No directory portion — no base.
    assert_eq!(relative_base("index.ts", "./x"), None);
}

#[test]
fn relative_file_matches_base_covers_extension_and_index_forms() {
    let base = "packages/q/src/queryClient";
    assert!(relative_file_matches_base(
        "packages/q/src/queryClient.ts",
        base
    ));
    // A deeper indexed path still matches as a suffix.
    assert!(relative_file_matches_base(
        "repo/packages/q/src/queryClient.tsx",
        base
    ));
    // The `/index` directory form of a parent base.
    assert!(relative_file_matches_base(
        "packages/q/src/index.ts",
        "packages/q/src"
    ));
    assert!(!relative_file_matches_base("packages/q/src/other.ts", base));
}

/// A specifier whose `relative_base` already carries its own non-TS/JS
/// extension (Dart's `foo.dart`, joined verbatim by `relative_base` since it
/// never strips extensions) matches the base literally — the bare-base
/// candidate — not only the TS/JS-extension-appended forms.
#[test]
fn extension_candidates_includes_bare_base_for_already_extensioned_specifiers() {
    let candidates = extension_candidates("lib/foo.dart");
    assert!(
        candidates.iter().any(|c| c == "lib/foo.dart"),
        "bare base must be a candidate: {candidates:?}"
    );
    assert!(relative_file_matches_base("lib/foo.dart", "lib/foo.dart"));
    assert!(relative_file_matches_base(
        "app/lib/foo.dart",
        "lib/foo.dart"
    ));
}

/// The bare-base addition does not disturb the existing TS/JS extension-guess
/// forms — an extension-less base still matches only through the appended
/// extension, not the (nonexistent) literal base file.
#[test]
fn extension_candidates_still_appends_ts_js_extensions_for_extensionless_base() {
    let candidates = extension_candidates("packages/q/src/queryClient");
    assert!(candidates.contains(&"packages/q/src/queryClient.ts".to_string()));
    assert!(candidates.contains(&"packages/q/src/queryClient/index.ts".to_string()));
    // The bare (extension-less) base is also present — harmless, since no
    // real TS/JS project has a source file with no extension at that path.
    assert!(candidates.contains(&"packages/q/src/queryClient".to_string()));
}
