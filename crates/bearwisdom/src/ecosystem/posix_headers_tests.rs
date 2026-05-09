use super::*;
use std::fs;
use tempfile::TempDir;

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn newest_sdk_version_picks_latest() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("10.0.22621.0/ucrt")).unwrap();
    fs::create_dir_all(tmp.path().join("10.0.26100.0/ucrt")).unwrap();
    fs::create_dir_all(tmp.path().join("wdf")).unwrap();
    let picked = newest_sdk_versions(tmp.path());
    assert_eq!(picked.len(), 1);
    assert!(picked[0].to_string_lossy().contains("10.0.26100.0"));
}

#[test]
fn newest_sdk_version_ignores_unversioned_siblings() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("10.0.26100.0")).unwrap();
    fs::create_dir_all(tmp.path().join("shared")).unwrap();
    fs::create_dir_all(tmp.path().join("wdf")).unwrap();
    let picked = newest_sdk_versions(tmp.path());
    assert_eq!(picked.len(), 1);
    assert!(picked[0].to_string_lossy().contains("10.0.26100.0"));
}

#[test]
fn header_index_registers_relative_path() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("stdio.h"), "int printf(const char*, ...);\n");
    write(&tmp.path().join("string.h"), "char* strcpy(char*, const char*);\n");
    let dep = make_root(tmp.path(), "test");
    let idx = build_c_header_index(&[dep]);
    assert!(!idx.is_empty());
    // `#include <stdio.h>` should locate stdio.h.
    assert!(idx.locate("stdio.h", "stdio.h").is_some());
    assert!(idx.locate("string.h", "string.h").is_some());
}

#[test]
fn header_index_registers_only_relative_path_from_root() {
    // Two roots cover the same SDK layout from different angles: one
    // mounted at `winrt/` (so `Windows.Foundation.h` is at the root)
    // and one mounted at the version dir above it (so the file is at
    // `winrt/Windows.Foundation.h`).  This is the real discover_msvc
    // emission pattern.  Both `#include` spellings should resolve —
    // not via basename fallback, but via the matching root.
    let tmp = TempDir::new().unwrap();
    let version_root = tmp.path();
    write(&version_root.join("winrt/Windows.Foundation.h"), "/* header */\n");
    let winrt_root = make_root(&version_root.join("winrt"), "test");
    let version_dep = make_root(version_root, "test");
    let idx = build_c_header_index(&[winrt_root, version_dep]);
    // Reached via the winrt/ root (relative = `Windows.Foundation.h`).
    assert!(idx.locate("Windows.Foundation.h", "Windows.Foundation.h").is_some());
    // Reached via the version root (relative = `winrt/Windows.Foundation.h`).
    assert!(idx.locate("winrt/Windows.Foundation.h", "winrt/Windows.Foundation.h").is_some());
}

#[test]
fn vcpkg_discovers_triplet_include_dirs() {
    // Lay out a fake vcpkg root with two triplets; only one with include/.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(&root.join("installed/x64-windows/include/openssl/bio.h"), "/* fake */\n");
    write(&root.join("installed/x64-linux/include/zlib.h"), "/* fake */\n");
    write(&root.join("installed/x86-windows/no-include-here.txt"), "ignored\n");

    std::env::set_var("VCPKG_ROOT", root);
    let dep_roots = discover_vcpkg_include();
    std::env::remove_var("VCPKG_ROOT");

    // Find the two triplet roots (x64-windows + x64-linux); x86-windows
    // is skipped because it has no include/.
    let triplet_dirs: Vec<String> = dep_roots
        .iter()
        .map(|r| r.root.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(
        triplet_dirs.iter().any(|p| p.ends_with("x64-windows/include")),
        "x64-windows/include not discovered; got {triplet_dirs:?}"
    );
    assert!(
        triplet_dirs.iter().any(|p| p.ends_with("x64-linux/include")),
        "x64-linux/include not discovered; got {triplet_dirs:?}"
    );
    assert!(
        !triplet_dirs.iter().any(|p| p.contains("x86-windows")),
        "x86-windows must be skipped (no include/); got {triplet_dirs:?}"
    );
}

#[test]
#[cfg(target_os = "windows")]
fn windows_header_index_registers_lowercase_shadow_key() {
    // Windows SDK has mixed-case header names like `WinSock2.h` but
    // user code always writes `#include <winsock2.h>`. The HashMap
    // backing SymbolLocationIndex is case-sensitive, so we need a
    // lowercase shadow key. Without it, demand-driven walking misses
    // these headers entirely.
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("WinSock2.h"), "/* mixed-case sdk header */\n");
    let dep = make_root(tmp.path(), "test");
    let idx = build_c_header_index(&[dep]);
    // Original case still resolves.
    assert!(idx.locate("WinSock2.h", "WinSock2.h").is_some());
    // Lowercase form (what `#include <winsock2.h>` generates) also
    // resolves to the same file.
    assert!(
        idx.locate("winsock2.h", "winsock2.h").is_some(),
        "lowercase shadow key required for case-insensitive Windows lookup"
    );
}

#[test]
fn header_index_does_not_basename_match_across_dirs() {
    // Regression: when only a deep-nested header exists, its basename
    // must NOT be registered — otherwise a user's project-local
    // `#include "async.h"` would wrongly pull a WinRT header.
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("wrl/async.h"), "/* winrt async */\n");
    let dep = make_root(tmp.path(), "test");
    let idx = build_c_header_index(&[dep]);
    assert!(idx.locate("wrl/async.h", "wrl/async.h").is_some());
    assert!(
        idx.locate("async.h", "async.h").is_none(),
        "basename-only lookup must not match a deeper-nested header",
    );
}

#[test]
fn header_index_skips_non_header_files() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("foo.h"), "\n");
    write(&tmp.path().join("README.md"), "docs\n");
    write(&tmp.path().join("license.txt"), "text\n");
    let dep = make_root(tmp.path(), "test");
    let idx = build_c_header_index(&[dep]);
    assert!(idx.locate("foo.h", "foo.h").is_some());
    assert!(idx.locate("README.md", "README.md").is_none());
    assert!(idx.locate("license.txt", "license.txt").is_none());
}

#[test]
fn resolve_header_finds_file_at_relative_path() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("stdio.h"), "\n");
    let dep = make_root(tmp.path(), "test");
    let found = resolve_header(&dep, "stdio.h").expect("should find stdio.h");
    assert!(found.absolute_path.ends_with("stdio.h"));
    assert_eq!(found.language, "c");
    assert!(found.relative_path.starts_with("ext:c:"));
}

#[test]
fn resolve_header_falls_back_to_basename() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("winrt/Windows.Foundation.h"), "\n");
    let dep = make_root(tmp.path(), "test");
    // Ask for just the basename (no directory prefix). Scanner should
    // walk the tree and find it.
    let found = resolve_header(&dep, "Windows.Foundation.h").expect("basename fallback");
    assert!(found.absolute_path.ends_with("Windows.Foundation.h"));
}

#[test]
fn resolve_header_returns_none_on_miss() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("foo.h"), "\n");
    let dep = make_root(tmp.path(), "test");
    assert!(resolve_header(&dep, "does-not-exist.h").is_none());
}

#[test]
fn posix_ecosystem_declares_demand_driven() {
    assert!(PosixHeadersEcosystem.uses_demand_driven_parse());
    assert!(PosixHeadersEcosystem.supports_reachability());
}

#[test]
fn posix_walk_root_is_empty_under_demand_driven() {
    let tmp = TempDir::new().unwrap();
    let dep = make_root(tmp.path(), POSIX_TAG);
    assert!(Ecosystem::walk_root(&PosixHeadersEcosystem, &dep).is_empty());
}

// ---------------------------------------------------------------------------
// Forwarding wrapper detection (Qt's `QObject` shape, recognised by content
// rather than name so the same logic generalises to any compile-DB-provided
// include root that happens to use the same idiom).
// ---------------------------------------------------------------------------

#[test]
fn parse_include_target_accepts_quoted_form() {
    assert_eq!(
        _test_parse_include_target("#include \"qobject.h\"\n").as_deref(),
        Some("qobject.h"),
    );
}

#[test]
fn parse_include_target_accepts_angle_form() {
    assert_eq!(
        _test_parse_include_target("#include <qobject.h>\n").as_deref(),
        Some("qobject.h"),
    );
}

#[test]
fn parse_include_target_skips_leading_whitespace_and_blank_lines() {
    assert_eq!(
        _test_parse_include_target("   \n\n  \t#include \"qobject.h\"\n").as_deref(),
        Some("qobject.h"),
    );
}

#[test]
fn parse_include_target_skips_line_and_block_comments() {
    let body = "// Generated by syncqt\n/* (C) Qt */\n#include \"qstringlist.h\"\n";
    assert_eq!(
        _test_parse_include_target(body).as_deref(),
        Some("qstringlist.h"),
    );
}

#[test]
fn parse_include_target_accepts_hash_space_include() {
    // `# include <x>` (with whitespace between # and include) is legal C.
    assert_eq!(
        _test_parse_include_target("# include <foo.h>\n").as_deref(),
        Some("foo.h"),
    );
}

#[test]
fn parse_include_target_rejects_non_include_first_token() {
    assert!(_test_parse_include_target("class QObject { };\n").is_none());
    assert!(_test_parse_include_target("This is a license file.\n").is_none());
    assert!(_test_parse_include_target("CC=cc\nall: foo\n").is_none());  // Makefile
    assert!(_test_parse_include_target("#define FOO 1\n").is_none());
    assert!(_test_parse_include_target("#pragma once\n").is_none());
}

#[test]
fn parse_include_target_rejects_unterminated_target() {
    assert!(_test_parse_include_target("#include \"qobject.h\n").is_none());
    assert!(_test_parse_include_target("#include <qobject.h\n").is_none());
}

#[test]
fn header_index_registers_extensionless_wrapper_files() {
    // Qt-style forwarding wrapper. Content is the discriminator; name pattern
    // is intentionally NOT checked, so this test uses a non-Q name to prove
    // the detection generalises.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(&root.join("Forward"), "#include \"forward.h\"\n");
    write(&root.join("forward.h"), "void forward(void);\n");
    let dep = make_root(root, "test");
    let idx = build_c_header_index(&[dep]);

    // `#include <Forward>` (target_name=module="Forward") resolves to a
    // file. The wrapper key MUST point at the real `.h`, not the
    // extensionless wrapper — the demand seed's `make_walked_file` does
    // language detection by extension and would silently drop the wrapper.
    let hit = idx.locate("Forward", "Forward").expect("wrapper must register");
    assert!(
        hit.to_string_lossy().ends_with("forward.h"),
        "wrapper key must point at the resolved real header, not the wrapper itself; got {hit:?}"
    );
    // `#include <forward.h>` still resolves to the real header.
    assert!(idx.locate("forward.h", "forward.h").is_some());
}

#[test]
fn header_index_registers_qt_style_module_wrapper() {
    // Realistic Qt layout: include/QtCore/QObject and include/QtCore/qobject.h.
    // The wrapper is at QtCore/QObject and content is `#include "qobject.h"`.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(&root.join("QtCore/QObject"), "#include \"qobject.h\"\n");
    write(&root.join("QtCore/qobject.h"), "class QObject {};\n");
    // Two roots — outer include/ and inner include/QtCore/, just like
    // KeePassXC's compile_commands feeds into build_c_header_index.
    let outer_dep = make_root(root, "test");
    let inner_dep = make_root(&root.join("QtCore"), "test");
    let idx = build_c_header_index(&[outer_dep, inner_dep]);

    // `#include <QObject>` (no module prefix) — the inner root makes this
    // resolve via the bare-basename wrapper key, pointing at qobject.h.
    let qobject_hit = idx.locate("QObject", "QObject").expect("QObject must resolve");
    assert!(
        qobject_hit.to_string_lossy().ends_with("qobject.h"),
        "bare `#include <QObject>` must resolve to qobject.h (not the wrapper); got {qobject_hit:?}"
    );
    // `#include <QtCore/QObject>` — outer root, relative path key. Also
    // points at the resolved real header.
    let module_hit = idx.locate("QtCore/QObject", "QtCore/QObject").expect("module-qualified must resolve");
    assert!(
        module_hit.to_string_lossy().ends_with("qobject.h"),
        "module-qualified `<QtCore/QObject>` must point at qobject.h; got {module_hit:?}"
    );
    // `#include <qobject.h>` — bare-name lowercase header via inner root.
    assert!(idx.locate("qobject.h", "qobject.h").is_some());
}

#[test]
fn header_index_skips_extensionless_non_wrapper_files() {
    // Files like LICENSE, COPYING, Makefile that happen to be extensionless
    // must NOT be registered as headers.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(&root.join("LICENSE"), "MIT License\n\nPermission is hereby granted...\n");
    write(&root.join("COPYING"), "GNU GPL v2\n");
    write(&root.join("Makefile"), "CC=cc\nall: foo\n\nfoo: foo.c\n");
    write(&root.join("README"), "This is the project readme.\n");
    let dep = make_root(root, "test");
    let idx = build_c_header_index(&[dep]);
    assert!(idx.locate("LICENSE", "LICENSE").is_none(),
        "LICENSE must not be registered as a header");
    assert!(idx.locate("COPYING", "COPYING").is_none());
    assert!(idx.locate("Makefile", "Makefile").is_none());
    assert!(idx.locate("README", "README").is_none());
}

#[test]
fn header_index_skips_oversized_extensionless_files() {
    // A 2KB extensionless file that happens to start with `#include` is
    // implausible as a wrapper (real wrappers are <100 bytes). The size
    // gate must reject it before the content check fires. Capitalized
    // name keeps it out of the C++ stdlib header path too.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let mut body = String::from("#include \"actual.h\"\n");
    while body.len() < 2048 {
        body.push_str("/* padding to exceed wrapper size cap */\n");
    }
    write(&root.join("BigForward"), &body);
    let dep = make_root(root, "test");
    let idx = build_c_header_index(&[dep]);
    assert!(idx.locate("BigForward", "BigForward").is_none(),
        "files larger than 1KB must not be detected as wrappers");
}

#[test]
fn header_index_accepts_cpp_stdlib_extensionless_headers() {
    // MSVC's `<vector>`, `<memory>`, `<string>` are extensionless files
    // containing the full template definitions — they fail both the
    // extension check and the wrapper size cap. The C++ stdlib name
    // pattern (lowercase identifier, no dots) recognizes them as real
    // headers.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let mut big_body = String::from("namespace std { template<class T> class vector { /* ... */ }; }\n");
    while big_body.len() < 2048 {
        big_body.push_str("// keep growing past the wrapper size cap\n");
    }
    write(&root.join("vector"), &big_body);
    write(&root.join("memory"), &big_body);
    write(&root.join("unordered_map"), &big_body);
    let dep = make_root(root, "test");
    let idx = build_c_header_index(&[dep]);
    assert!(
        idx.locate("vector", "vector").is_some(),
        "C++ stdlib `vector` must be indexed"
    );
    assert!(idx.locate("memory", "memory").is_some());
    assert!(
        idx.locate("unordered_map", "unordered_map").is_some(),
        "underscore-bearing stdlib names must be accepted"
    );
}

#[test]
fn cpp_stdlib_header_name_predicate() {
    use super::_test_is_cpp_stdlib_header_name as is_stdlib;
    // Real stdlib headers — accepted.
    assert!(is_stdlib("vector"));
    assert!(is_stdlib("memory"));
    assert!(is_stdlib("unordered_map"));
    assert!(is_stdlib("string_view"));
    assert!(is_stdlib("condition_variable"));
    assert!(is_stdlib("cstdio"));
    assert!(is_stdlib("array"));
    // Capitalized — rejected.
    assert!(!is_stdlib("LICENSE"));
    assert!(!is_stdlib("README"));
    assert!(!is_stdlib("Makefile"));
    assert!(!is_stdlib("BigForward"));
    assert!(!is_stdlib("QObject"));
    // Has dot — rejected.
    assert!(!is_stdlib("version.in"));
    assert!(!is_stdlib("config.h"));
    // Empty / single char — rejected.
    assert!(!is_stdlib(""));
    assert!(!is_stdlib("a"));
    // Starts with digit / underscore — rejected (not a stdlib shape).
    assert!(!is_stdlib("1vector"));
    assert!(!is_stdlib("_internal"));
    // Has dash — rejected.
    assert!(!is_stdlib("some-file"));
}
