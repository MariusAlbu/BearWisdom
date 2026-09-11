use super::*;

#[test]
fn explicit_relative_include_prefers_the_source_directory() {
    let files = [
        ("src/main.c", "c"),
        ("src/local.h", "c"),
        ("ext:idx:/sdk/local.h", "c"),
    ];
    assert_eq!(
        resolve("src/main.c", "\"./local.h\"", &files).as_deref(),
        Some("src/local.h")
    );
    assert_eq!(resolve("src/main.c", "\"./missing.h\"", &files), None);
}

#[test]
fn quoted_include_searches_the_source_directory_first() {
    let files = [
        ("src/main.c", "c"),
        ("src/stdio.h", "c"),
        ("ext:idx:/sdk/include/stdio.h", "c"),
    ];
    assert_eq!(
        resolve("src/main.c", "\"stdio.h\"", &files).as_deref(),
        Some("src/stdio.h")
    );
}

#[test]
fn quoted_include_falls_back_to_a_unique_project_file_then_external() {
    let files = [
        ("lib/url.c", "c"),
        ("lib/curl_setup.h", "c"),
        ("include/curl/curl.h", "c"),
        ("ext:idx:/sdk/include/string.h", "c"),
    ];
    assert_eq!(
        resolve("lib/url.c", "\"curl_setup.h\"", &files).as_deref(),
        Some("lib/curl_setup.h")
    );
    assert_eq!(
        resolve("lib/url.c", "\"curl/curl.h\"", &files).as_deref(),
        Some("include/curl/curl.h")
    );
    assert_eq!(
        resolve("lib/url.c", "\"string.h\"", &files).as_deref(),
        Some("ext:idx:/sdk/include/string.h")
    );
}

#[test]
fn angled_include_prefers_external_then_a_unique_project_root() {
    let files = [
        ("lib/url.c", "c"),
        ("tests/stdio.h", "c"),
        ("include/curl/curl.h", "c"),
        ("ext:idx:/sdk/include/stdio.h", "c"),
    ];
    assert_eq!(
        resolve("lib/url.c", "<stdio.h>", &files).as_deref(),
        Some("ext:idx:/sdk/include/stdio.h")
    );
    assert_eq!(
        resolve("lib/url.c", "<curl/curl.h>", &files).as_deref(),
        Some("include/curl/curl.h")
    );
}

#[test]
fn ambiguous_matches_place_nothing() {
    let files = [
        ("a/main.c", "c"),
        ("b/config.h", "c"),
        ("c/config.h", "c"),
        ("ext:idx:/sdk/one/x.h", "c"),
        ("ext:idx:/sdk/two/x.h", "c"),
    ];
    assert_eq!(resolve("a/main.c", "\"config.h\"", &files), None);
    assert_eq!(resolve("a/main.c", "<x.h>", &files), None);
}

#[test]
fn undelimited_spelling_places_only_a_unique_external_file() {
    let files = [
        ("src/main.c", "c"),
        ("src/stdio.h", "c"),
        ("src/main.h", "c"),
        ("ext:idx:/sdk/include/stdio.h", "c"),
    ];
    assert_eq!(resolve("src/main.c", "stdio.h", &files), None);
    assert_eq!(resolve("src/main.c", "main.h", &files), None);
    assert_eq!(
        resolve("src/main.c", "unistd.h", &[("src/main.c", "c"), ("ext:idx:/sdk/unistd.h", "c")])
            .as_deref(),
        Some("ext:idx:/sdk/unistd.h")
    );
    assert_eq!(resolve("src/main.c", "\"main.c\"", &files), None);
    assert_eq!(resolve("src/main.c", "<>", &files), None);
}

#[test]
fn c_sources_include_headers_the_detector_tags_as_cpp() {
    let files = [
        ("lib/url.c", "c"),
        ("include/curl/curl.h", "cpp"),
        ("ext:idx:/sdk/include/stdio.h", "c"),
    ];
    assert_eq!(
        resolve("lib/url.c", "<curl/curl.h>", &files).as_deref(),
        Some("include/curl/curl.h")
    );
    assert_eq!(
        resolve("lib/url.c", "\"curl/curl.h\"", &files).as_deref(),
        Some("include/curl/curl.h")
    );
    assert_eq!(resolve("docs/a.py", "<curl/curl.h>", &[("docs/a.py", "python"), ("include/curl/curl.h", "cpp")]), None);
}
