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
