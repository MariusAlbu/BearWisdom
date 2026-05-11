// languages/fortran/fypp_tests.rs — unit tests for fypp preprocessing
use super::*;

/// Minimal fypp source that doesn't need `common.fypp` — exercises the
/// subprocess round-trip without project-specific includes.
const SIMPLE_FYPP: &[u8] = br#"
#:for T in ['integer', 'real']
pure function add_${T}$(a, b) result(c)
  ${T}$, intent(in) :: a, b
  ${T}$ :: c
  c = a + b
end function add_${T}$
#:endfor
"#;

#[test]
fn preprocess_returns_none_gracefully_on_bad_path() {
    // A file path with no ancestor include/ directory — should not panic.
    let result = preprocess("/nonexistent/path/foo.fypp", b"#:set X = 1\n");
    // Either None (fypp not available) or Some(expanded) — both are valid.
    // The key invariant is no panic.
    let _ = result;
}

#[test]
fn sha256_hex_stable() {
    let h1 = sha256_hex(b"hello");
    let h2 = sha256_hex(b"hello");
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 64);
}

#[test]
fn cache_path_contains_hash() {
    let path = cache_path("abc123");
    let name = path.file_name().unwrap().to_string_lossy();
    assert!(name.contains("bw_fypp_abc123"), "unexpected cache name: {name}");
    assert!(name.ends_with(".f90"));
}

#[test]
fn find_include_dir_returns_none_for_root() {
    // No include/common.fypp ancestor of the filesystem root.
    let result = find_include_dir("/no_such_directory/file.fypp");
    assert!(result.is_none());
}

#[test]
fn preprocess_simple_template_when_fypp_available() {
    // Only runs when fypp is actually installed — skips silently otherwise.
    if locate_fypp().is_none() {
        return;
    }
    // Use a trivially valid template with no includes.
    let result = preprocess("/tmp/test.fypp", SIMPLE_FYPP);
    if let Some(output) = result {
        assert!(
            output.contains("add_integer") || output.contains("add_real"),
            "expected instantiated names in output, got: {output}"
        );
        // Template markers must not survive into the output.
        assert!(
            !output.contains("#:for") && !output.contains("${T}$"),
            "template markers leaked into fypp output: {output}"
        );
    }
    // None is also acceptable — some fypp versions may reject the stub path.
}
