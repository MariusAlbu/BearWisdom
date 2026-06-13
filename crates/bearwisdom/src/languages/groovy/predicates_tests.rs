// Tests for `is_groovy_builtin` were removed when the predicate itself
// was deleted (DGM/GDK Object-mixin names moved to groovy/keywords.rs;
// JVM types covered by jdk_src + groovy_stdlib walkers).

use super::predicates::is_interpolation_marker;

#[test]
fn interpolation_marker_matches_dollar_and_empty() {
    assert!(is_interpolation_marker("$"));
    assert!(is_interpolation_marker(""));
}

#[test]
fn interpolation_marker_rejects_real_identifiers() {
    assert!(!is_interpolation_marker("fieldValue"));
    assert!(!is_interpolation_marker("name"));
    // `$`-containing identifiers are real names, not the bare marker.
    assert!(!is_interpolation_marker("$scope"));
    assert!(!is_interpolation_marker("foo$bar"));
}
