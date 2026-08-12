use super::virtual_path_for_pulled;
use std::path::Path;

#[test]
fn rust_pulled_file_matches_eager_walker_shape() {
    // A demand-pulled cargo registry file must reconstruct the eager walker's
    // `ext:rust:<crate>/<rel-to-crate-root>` virtual path (version stripped),
    // so the `already_walked` dedupe recognizes a re-pulled file.
    let abs = Path::new(
        "/home/u/.cargo/registry/src/index.crates.io-abc/windows-0.61.3/src/Windows/Win32/mod.rs",
    );
    assert_eq!(
        virtual_path_for_pulled(abs, "rust").as_deref(),
        Some("ext:rust:windows/src/Windows/Win32/mod.rs"),
    );
}

#[test]
fn rust_pulled_file_handles_hyphenated_crate() {
    let abs = Path::new("/home/u/.cargo/registry/src/index-1/proc-macro2-1.0.91/src/lib.rs");
    assert_eq!(
        virtual_path_for_pulled(abs, "rust").as_deref(),
        Some("ext:rust:proc-macro2/src/lib.rs"),
    );
}

#[test]
fn rust_pulled_file_windows_separators() {
    let abs = Path::new(r"C:\Users\u\.cargo\registry\src\index-1\serde-1.0.200\src\de\mod.rs");
    assert_eq!(
        virtual_path_for_pulled(abs, "rust").as_deref(),
        Some("ext:rust:serde/src/de/mod.rs"),
    );
}

#[test]
fn rust_pulled_file_non_registry_path_falls_through() {
    // Path outside the registry layout can't be shaped — caller falls back to
    // `ext:idx:`.
    let abs = Path::new("/some/where/else/foo.rs");
    assert_eq!(virtual_path_for_pulled(abs, "rust"), None);
}

#[test]
fn ruby_pulled_file_matches_eager_walker_shape() {
    // A demand-pulled RubyGems file must reconstruct the eager walker's
    // `ext:ruby:<gem>/<rel>` virtual path (version stripped), so the
    // `already_walked` dedupe recognizes a re-pulled file and
    // `ExtMatch::PkgSegment` can read the gem name back out.
    let abs = Path::new(
        "/home/u/.local/share/gem/ruby/3.3.0/gems/actionpack-8.1.3/lib/abstract_controller.rb",
    );
    assert_eq!(
        virtual_path_for_pulled(abs, "ruby").as_deref(),
        Some("ext:ruby:actionpack/lib/abstract_controller.rb"),
    );
}

#[test]
fn ruby_pulled_file_handles_hyphenated_gem() {
    let abs = Path::new(
        "/home/u/.local/share/gem/ruby/3.3.0/gems/rails-html-sanitizer-1.0.0/lib/rails/html/sanitizer.rb",
    );
    assert_eq!(
        virtual_path_for_pulled(abs, "ruby").as_deref(),
        Some("ext:ruby:rails-html-sanitizer/lib/rails/html/sanitizer.rb"),
    );
}

#[test]
fn ruby_pulled_file_handles_platform_suffixed_version() {
    let abs =
        Path::new("/home/u/.local/share/gem/ruby/3.3.0/gems/ffi-1.17.4-x64-mingw-ucrt/lib/ffi.rb");
    assert_eq!(
        virtual_path_for_pulled(abs, "ruby").as_deref(),
        Some("ext:ruby:ffi/lib/ffi.rb"),
    );
}

#[test]
fn ruby_pulled_file_windows_separators() {
    let abs = Path::new(
        r"C:\Users\u\.local\share\gem\ruby\3.3.0\gems\devise-4.9.3\lib\devise\version.rb",
    );
    assert_eq!(
        virtual_path_for_pulled(abs, "ruby").as_deref(),
        Some("ext:ruby:devise/lib/devise/version.rb"),
    );
}

#[test]
fn dart_pulled_flutter_package_file_matches_eager_walker_shape() {
    let abs = Path::new(
        r"C:\flutter\packages\flutter\lib\src\widgets\framework.dart",
    );
    assert_eq!(
        virtual_path_for_pulled(abs, "dart").as_deref(),
        Some("ext:flutter-sdk:flutter/lib/src/widgets/framework.dart"),
    );
}

#[test]
fn dart_pulled_pub_cache_file_matches_eager_walker_shape() {
    // A demand-pulled pub-cache package file must reconstruct the eager
    // `PubEcosystem` walker's `ext:dart:<pkg>/<rel>` shape (version stripped,
    // `rel` relative to the package's `lib/` dir), so the `already_walked`
    // dedupe recognizes a re-pulled file.
    let abs = Path::new("/home/u/.pub-cache/hosted/pub.dev/collection-1.18.0/lib/collection.dart");
    assert_eq!(
        virtual_path_for_pulled(abs, "dart").as_deref(),
        Some("ext:dart:collection/collection.dart"),
    );
}

#[test]
fn dart_pulled_pub_cache_file_with_subdir() {
    let abs = Path::new(
        "/home/u/.pub-cache/hosted/pub.dev/riverpod-2.4.0/lib/src/framework.dart",
    );
    assert_eq!(
        virtual_path_for_pulled(abs, "dart").as_deref(),
        Some("ext:dart:riverpod/src/framework.dart"),
    );
}

#[test]
fn dart_pulled_pub_cache_file_windows_separators() {
    let abs = Path::new(
        r"C:\Users\u\AppData\Local\Pub\Cache\hosted\pub.dev\collection-1.18.0\lib\collection.dart",
    );
    assert_eq!(
        virtual_path_for_pulled(abs, "dart").as_deref(),
        Some("ext:dart:collection/collection.dart"),
    );
}

#[test]
fn dart_pulled_dart_sdk_file_matches_eager_walker_shape() {
    // A demand-pulled Dart-SDK file must reconstruct the eager
    // `DartSdkEcosystem` walker's `ext:dart-sdk:<lib>/<rel>` shape
    // (dart_sdk.rs — `rel` relative to the SDK's `lib/` dir), recognized by
    // the sub-library name rather than an install-root literal.
    let abs = Path::new(r"C:\Program Files\Dart\dart-sdk\lib\core\core.dart");
    assert_eq!(
        virtual_path_for_pulled(abs, "dart").as_deref(),
        Some("ext:dart-sdk:core/core.dart"),
    );
}

#[test]
fn dart_pulled_dart_sdk_file_unix_install() {
    let abs = Path::new("/usr/lib/dart/lib/async/async.dart");
    assert_eq!(
        virtual_path_for_pulled(abs, "dart").as_deref(),
        Some("ext:dart-sdk:async/async.dart"),
    );
}

#[test]
fn dart_pulled_project_lib_file_falls_through() {
    // A project's own `lib/main.dart` is not under a recognized SDK
    // sub-library — no walker scheme to agree with.
    let abs = Path::new("/home/u/myproject/lib/main.dart");
    assert_eq!(virtual_path_for_pulled(abs, "dart"), None);
}

#[test]
fn elixir_pulled_stdlib_file_gets_stdlib_ecosystem_segment() {
    // The `elixir-stdlib` segment is what `is_ambient_global_lib_path`
    // classifies as ambient scope — bare `Enum`/`Map` binding depends on it.
    let abs = Path::new(
        r"C:\Users\u\scoop\apps\elixir\current\lib\ex_unit\lib\ex_unit\case_template.ex",
    );
    assert_eq!(
        virtual_path_for_pulled(abs, "elixir").as_deref(),
        Some("ext:elixir-stdlib:ex_unit/ex_unit/case_template.ex"),
    );
}

#[test]
fn elixir_pulled_stdlib_core_module() {
    let abs = Path::new("/usr/local/lib/elixir/lib/elixir/lib/enum.ex");
    assert_eq!(
        virtual_path_for_pulled(abs, "elixir").as_deref(),
        Some("ext:elixir-stdlib:elixir/enum.ex"),
    );
}

#[test]
fn elixir_pulled_hex_dep_matches_hex_walker_shape() {
    // Mix vendors deps in-project; the pkg-segmented `ext:elixir:` form is the
    // hex walker's own shape, so PkgSegment reads the package back out.
    let abs = Path::new("/home/u/proj/deps/phoenix/lib/phoenix/router.ex");
    assert_eq!(
        virtual_path_for_pulled(abs, "elixir").as_deref(),
        Some("ext:elixir:phoenix/lib/phoenix/router.ex"),
    );
}

#[test]
fn elixir_pulled_non_layout_path_falls_through() {
    let abs = Path::new("/some/where/else/foo.ex");
    assert_eq!(virtual_path_for_pulled(abs, "elixir"), None);
}

#[test]
fn ruby_pulled_file_non_gems_path_falls_through() {
    // Path outside the `gems/` layout can't be shaped — caller falls back to
    // `ext:idx:`.
    let abs = Path::new("/some/where/else/foo.rb");
    assert_eq!(virtual_path_for_pulled(abs, "ruby"), None);
}
