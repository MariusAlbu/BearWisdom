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
fn go_pulled_goroot_stdlib_file_matches_eager_walker_shape() {
    let abs = Path::new(r"C:\Program Files\Go\src\net\http\server.go");
    assert_eq!(
        virtual_path_for_pulled(abs, "go").as_deref(),
        Some("ext:go-stdlib/net/http/server.go"),
    );
}

#[test]
fn go_pulled_module_cache_still_wins_over_src_marker() {
    // A module-cache path containing `/src/` inside the module tree must keep
    // the `ext:go/` module-cache identity, not the stdlib shape.
    let abs = Path::new("/home/u/go/pkg/mod/github.com/gofiber/fiber@v2.52.0/src/helpers.go");
    assert_eq!(
        virtual_path_for_pulled(abs, "go").as_deref(),
        Some("ext:go/github.com/gofiber/fiber@v2.52.0/src/helpers.go"),
    );
}

#[test]
fn go_pulled_non_layout_path_falls_through() {
    // Neither `/pkg/mod/` nor `/src/` present — caller falls back to `ext:idx:`.
    let abs = Path::new("/some/where/else/foo.go");
    assert_eq!(virtual_path_for_pulled(abs, "go"), None);
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

#[test]
fn ruby_pulled_stdlib_file_gets_stdlib_ecosystem_segment() {
    // `RbConfig::CONFIG['rubylibdir']` layout — no `/gems/` segment. The
    // `ruby-stdlib` segment is what `is_ambient_global_lib_path` classifies
    // as ambient scope, letting a bare `JSON` (no `module` tag survives past
    // the `require` ref itself) bind through the ambient_scope rung.
    let abs = Path::new("/usr/lib/ruby/3.3.0/json.rb");
    assert_eq!(
        virtual_path_for_pulled(abs, "ruby").as_deref(),
        Some("ext:ruby-stdlib:json.rb"),
    );
}

#[test]
fn ruby_pulled_stdlib_file_with_subdir() {
    let abs = Path::new("/usr/lib/ruby/3.3.0/net/http.rb");
    assert_eq!(
        virtual_path_for_pulled(abs, "ruby").as_deref(),
        Some("ext:ruby-stdlib:net/http.rb"),
    );
}

#[test]
fn ruby_pulled_stdlib_file_windows_separators() {
    let abs = Path::new(r"C:\Ruby33-x64\lib\ruby\3.3.0\digest.rb");
    assert_eq!(
        virtual_path_for_pulled(abs, "ruby").as_deref(),
        Some("ext:ruby-stdlib:digest.rb"),
    );
}

#[test]
fn ruby_pulled_gems_path_wins_over_stdlib_marker() {
    // A gem path never contains `/lib/ruby/`, but this guards the ordering
    // intent: `/gems/` is checked first so a real gem is never misclassified
    // as stdlib.
    let abs = Path::new(
        "/home/u/.local/share/gem/ruby/3.3.0/gems/actionpack-8.1.3/lib/abstract_controller.rb",
    );
    assert_eq!(
        virtual_path_for_pulled(abs, "ruby").as_deref(),
        Some("ext:ruby:actionpack/lib/abstract_controller.rb"),
    );
}

#[test]
fn pascal_pulled_system_pp_gets_stdlib_ecosystem_segment() {
    // `system.pp` is System's own unit body — implicit in every Pascal file,
    // no `uses System;` ever appears, so it must resolve through the
    // ambient_scope rung.
    let abs = Path::new(
        r"C:\Users\u\scoop\apps\lazarus\current\fpc\3.2.2\source\rtl\win64\system.pp",
    );
    assert_eq!(
        virtual_path_for_pulled(abs, "pascal").as_deref(),
        Some("ext:fpc-stdlib:system/system.pp"),
    );
}

#[test]
fn pascal_pulled_rtl_inc_system_fragment_gets_stdlib_segment() {
    // `objpash.inc` declares `TObject` and is spliced into every platform's
    // `system.pp` via `systemh.inc` — confirmed by tracing the FPC 3.2.2
    // `{$I}` graph.
    let abs = Path::new("/home/u/lazarus/fpc/3.2.2/source/rtl/inc/objpash.inc");
    assert_eq!(
        virtual_path_for_pulled(abs, "pascal").as_deref(),
        Some("ext:fpc-stdlib:system/objpash.inc"),
    );
}

#[test]
fn pascal_pulled_rtl_inc_dos_fragment_stays_non_ambient() {
    // `dos.inc`/`dosh.inc` splice into the `Dos` unit's own `dos.pp` files
    // (per-platform), never into `system.pp` — `Dos` still requires an
    // explicit `uses Dos;`, so tagging it ambient would let `GetDate`/
    // `DiskFree` bind without one.
    let abs = Path::new("/home/u/lazarus/fpc/3.2.2/source/rtl/inc/dos.inc");
    assert_eq!(
        virtual_path_for_pulled(abs, "pascal").as_deref(),
        Some("ext:fpc:fpc-rtl-inc/dos.inc"),
    );
}

#[test]
fn pascal_pulled_rtl_inc_standalone_unit_stays_non_ambient() {
    // `strings.pp` declares `unit Strings;` — a real standalone unit
    // requiring `uses Strings;`, not a fragment spliced into System.
    let abs = Path::new("/home/u/lazarus/fpc/3.2.2/source/rtl/inc/strings.pp");
    assert_eq!(
        virtual_path_for_pulled(abs, "pascal").as_deref(),
        Some("ext:fpc:fpc-rtl-inc/strings.pp"),
    );
}

#[test]
fn pascal_pulled_objpas_unit_stays_non_ambient() {
    // `SysUtils`/`Classes` live under `rtl/objpas/` — a project must still
    // write `uses SysUtils;`; the `WildcardMatch::FileStem` rung already
    // opens them on that clause, so they must not be ambient.
    let abs = Path::new("/home/u/lazarus/fpc/3.2.2/source/rtl/objpas/sysutils/sysutils.pp");
    assert_eq!(
        virtual_path_for_pulled(abs, "pascal").as_deref(),
        Some("ext:fpc:fpc-rtl-objpas/sysutils/sysutils.pp"),
    );
}

#[test]
fn pascal_pulled_lcl_file_stays_non_ambient() {
    let abs = Path::new(r"C:\lazarus\lcl\forms.pp");
    assert_eq!(
        virtual_path_for_pulled(abs, "pascal").as_deref(),
        Some("ext:fpc:lcl/forms.pp"),
    );
}

#[test]
fn pascal_pulled_lazarus_component_stays_non_ambient() {
    let abs = Path::new("/home/u/lazarus/components/codetools/codetoolsstrconsts.pas");
    assert_eq!(
        virtual_path_for_pulled(abs, "pascal").as_deref(),
        Some("ext:fpc:lazarus-components/codetools/codetoolsstrconsts.pas"),
    );
}

#[test]
fn pascal_pulled_fpc_package_stays_non_ambient() {
    let abs = Path::new("/home/u/lazarus/fpc/3.2.2/source/packages/fcl-json/src/fpjson.pp");
    assert_eq!(
        virtual_path_for_pulled(abs, "pascal").as_deref(),
        Some("ext:fpc:fpc-pkg-fcl-json/fpjson.pp"),
    );
}

#[test]
fn pascal_pulled_shared_rtl_dir_unit_stays_non_ambient() {
    // `Dos` on the shared `win` target dir — a real standalone unit, not a
    // System splice.
    let abs = Path::new("/home/u/lazarus/fpc/3.2.2/source/rtl/win/dos.pp");
    assert_eq!(
        virtual_path_for_pulled(abs, "pascal").as_deref(),
        Some("ext:fpc:fpc-rtl-win/dos.pp"),
    );
}

#[test]
fn pascal_pulled_non_layout_path_falls_through() {
    let abs = Path::new("/some/where/else/foo.pas");
    assert_eq!(virtual_path_for_pulled(abs, "pascal"), None);
}
