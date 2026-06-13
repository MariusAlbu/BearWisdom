use super::*;
use std::path::PathBuf;

/// Build a directory tree from a list of relative dir paths under `root`.
fn make_dirs(root: &PathBuf, dirs: &[&str]) {
    for d in dirs {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
}

#[test]
fn zig_toolchain_marks_libc_and_libcxx_subtrees() {
    let tmp = std::env::temp_dir().join("bw-test-toolchain-zig");
    let _ = std::fs::remove_dir_all(&tmp);
    // Zig compiler checkout layout: lib/std is the toolchain marker;
    // lib/libc*, lib/libcxx* are the vendored C/C++ payload.
    make_dirs(
        &tmp,
        &["lib/std", "lib/libc", "lib/libcxx", "lib/libcxxabi", "src"],
    );

    let prefixes = toolchain_payload_prefixes(&tmp);
    assert!(
        prefixes.contains(&"lib/libc".to_string()),
        "lib/libc payload missing; got: {prefixes:?}"
    );
    assert!(
        prefixes.contains(&"lib/libcxx".to_string()),
        "lib/libcxx payload missing; got: {prefixes:?}"
    );
    assert!(
        prefixes.contains(&"lib/libcxxabi".to_string()),
        "lib/libcxxabi payload missing; got: {prefixes:?}"
    );

    // Files under the payload subtrees are external.
    assert!(is_under_toolchain_payload(
        "lib/libc/musl/src/string/memcpy.c",
        &prefixes
    ));
    assert!(is_under_toolchain_payload(
        "lib/libcxx/src/vector.cpp",
        &prefixes
    ));
    // The Zig stdlib (lib/std) and the compiler's own src stay internal —
    // lib/std is indexed as the zig-std ecosystem, not toolchain payload.
    assert!(!is_under_toolchain_payload(
        "lib/std/mem.zig",
        &prefixes
    ));
    assert!(!is_under_toolchain_payload("src/main.zig", &prefixes));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn zig_toolchain_marks_bundled_clang_headers_and_tsan_runtime() {
    let tmp = std::env::temp_dir().join("bw-test-toolchain-zig-clang");
    let _ = std::fs::remove_dir_all(&tmp);
    // lib/include (bundled clang headers) and lib/libtsan (the compiler-rt
    // ThreadSanitizer runtime) are vendored LLVM/clang payload, not Zig code.
    make_dirs(
        &tmp,
        &["lib/std", "lib/include", "lib/libtsan", "lib/compiler", "src"],
    );

    let prefixes = toolchain_payload_prefixes(&tmp);
    assert!(
        prefixes.contains(&"lib/include".to_string()),
        "lib/include (clang headers) payload missing; got: {prefixes:?}"
    );
    assert!(
        prefixes.contains(&"lib/libtsan".to_string()),
        "lib/libtsan (compiler-rt) payload missing; got: {prefixes:?}"
    );

    assert!(is_under_toolchain_payload(
        "lib/include/stddef.h",
        &prefixes
    ));
    assert!(is_under_toolchain_payload(
        "lib/libtsan/tsan_rtl.cpp",
        &prefixes
    ));
    // Zig's own compiler implementation under lib/ stays internal.
    assert!(!is_under_toolchain_payload(
        "lib/compiler/aro/aro.zig",
        &prefixes
    ));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn non_zig_project_with_lib_dir_is_not_a_toolchain() {
    let tmp = std::env::temp_dir().join("bw-test-toolchain-not-zig");
    let _ = std::fs::remove_dir_all(&tmp);
    // A regular app with a lib/ dir but no lib/std marker — not a toolchain.
    make_dirs(&tmp, &["lib/libc", "src"]);

    let prefixes = toolchain_payload_prefixes(&tmp);
    assert!(
        prefixes.is_empty(),
        "non-toolchain project must not classify lib/libc as payload; got: {prefixes:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn odin_toolchain_marks_core_and_vendor() {
    let tmp = std::env::temp_dir().join("bw-test-toolchain-odin");
    let _ = std::fs::remove_dir_all(&tmp);
    // Odin compiler checkout layout: core/ + vendor/ at the root, with the
    // compiler's own src/.
    make_dirs(&tmp, &["core", "vendor", "base", "src", "examples"]);

    let prefixes = toolchain_payload_prefixes(&tmp);
    assert!(
        prefixes.contains(&"core".to_string()),
        "core payload missing; got: {prefixes:?}"
    );
    assert!(
        prefixes.contains(&"vendor".to_string()),
        "vendor payload missing; got: {prefixes:?}"
    );

    assert!(is_under_toolchain_payload(
        "core/fmt/fmt.odin",
        &prefixes
    ));
    assert!(is_under_toolchain_payload(
        "vendor/raylib/raylib.odin",
        &prefixes
    ));
    // The compiler's own sources stay internal.
    assert!(!is_under_toolchain_payload("src/main.odin", &prefixes));
    assert!(!is_under_toolchain_payload(
        "examples/demo.odin",
        &prefixes
    ));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn odin_app_without_both_core_and_vendor_is_not_a_toolchain() {
    let tmp = std::env::temp_dir().join("bw-test-toolchain-odin-app");
    let _ = std::fs::remove_dir_all(&tmp);
    // An Odin app may have a core/ dir of its own but not vendor/ — the
    // toolchain signal requires both, so this is not the toolchain payload.
    make_dirs(&tmp, &["core", "src"]);

    let prefixes = toolchain_payload_prefixes(&tmp);
    assert!(
        prefixes.is_empty(),
        "core/ alone must not be classified as payload; got: {prefixes:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn under_payload_matches_subtree_not_prefix_sibling() {
    let prefixes = vec!["lib/libc".to_string(), "core".to_string()];
    assert!(is_under_toolchain_payload("lib/libc/x.c", &prefixes));
    assert!(is_under_toolchain_payload("core", &prefixes)); // the declared dir itself
    assert!(!is_under_toolchain_payload(
        "lib/libcxx/x.cpp",
        &prefixes
    )); // not a declared prefix
    assert!(!is_under_toolchain_payload("corelib/x.odin", &prefixes)); // prefix sibling
}

#[test]
fn windows_backslash_paths_normalize() {
    let prefixes = vec!["lib/libc".to_string()];
    assert!(is_under_toolchain_payload(
        "lib\\libc\\musl\\x.c",
        &prefixes
    ));
}

#[test]
fn empty_prefix_set_matches_nothing() {
    assert!(!is_under_toolchain_payload("anything/at/all.c", &[]));
}
