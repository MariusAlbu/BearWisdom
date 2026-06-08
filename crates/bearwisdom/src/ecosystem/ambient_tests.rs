// =============================================================================
// ecosystem/ambient_tests.rs — framework ambient path-marker matching.
// =============================================================================

use super::*;

fn norm(p: &str) -> String {
    p.to_lowercase().replace('\\', "/")
}

#[test]
fn matches_nuxt_and_svelte_and_next_generated() {
    for p in [
        "project/.nuxt/imports.d.ts",
        "project/.nuxt/components.d.ts",
        "app/.svelte-kit/ambient.d.ts",
        "web/.next/next-env.d.ts",
    ] {
        assert!(is_framework_ambient_path(&norm(p)), "should be ambient: {p}");
    }
}

#[test]
fn matches_root_relative_forms() {
    for p in [
        ".nuxt/imports.d.ts",
        ".svelte-kit/ambient.d.ts",
        ".next/next-env.d.ts",
    ] {
        assert!(is_framework_ambient_path(&norm(p)), "root-relative should match: {p}");
    }
}

#[test]
fn matches_vue_runtime_declarations() {
    for p in [
        "node_modules/vue/dist/vue.d.ts",
        "node_modules/@vue/runtime-core/dist/runtime-core.d.ts",
        "node_modules/@vue/runtime-dom/dist/runtime-dom.d.ts",
        "node_modules/@vue/reactivity/dist/reactivity.d.ts",
    ] {
        assert!(is_framework_ambient_path(&norm(p)), "vue runtime should match: {p}");
    }
}

#[test]
fn matches_bicep_runtime_path() {
    // The synthetic runtime file the bicep-runtime ecosystem emits.
    assert!(
        is_framework_ambient_path(&norm("ext:bicep-runtime:namespace.bicep")),
        "bicep runtime path should match",
    );
}

#[test]
fn matches_bazel_builtins_path() {
    // The synthetic built-in / ctx / env files the bazel-central-registry
    // ecosystem emits.
    for p in [
        "ext:bazel-builtins:rules.bzl",
        "ext:bazel-builtins:ctx.bzl",
        "ext:bazel-builtins:env.bzl",
    ] {
        assert!(is_framework_ambient_path(&norm(p)), "bazel builtin should match: {p}");
    }
}

#[test]
fn matches_rust_stdlib_prelude_sources() {
    // The sysroot source tree the rust-stdlib walker keys prelude symbols under.
    for p in [
        "ext:rust:C:/Users/x/.rustup/toolchains/stable/lib/rustlib/src/rust/library/alloc/src/vec/mod.rs",
        "ext:rust:/home/u/.rustup/.../lib/rustlib/src/rust/library/core/src/option.rs",
        "ext:rust:/home/u/.rustup/.../lib/rustlib/src/rust/library/std/src/lib.rs",
    ] {
        assert!(is_framework_ambient_path(&norm(p)), "stdlib prelude source should match: {p}");
    }
}

#[test]
fn matches_dart_core_and_rejects_other_dart_libs() {
    for p in [
        "ext:idx:C:/x/flutter/cache/dart-sdk/lib/core/errors.dart",
        "ext:idx:C:/x/flutter/cache/pkg/sky_engine/lib/core/list.dart",
    ] {
        assert!(is_framework_ambient_path(&norm(p)), "dart:core should be ambient: {p}");
    }
    // Other dart: libraries need an explicit import — not ambient.
    for p in [
        "ext:idx:C:/x/flutter/cache/dart-sdk/lib/async/stream.dart",
        "ext:idx:C:/x/Pub/Cache/hosted/pub.dev/googleapis-16.0.0/lib/docs/v1.dart",
    ] {
        assert!(!is_framework_ambient_path(&norm(p)), "non-core dart lib must NOT be ambient: {p}");
    }
}

#[test]
fn matches_haskell_ghc_internal_prelude() {
    for p in [
        "ext:haskell:ghc-internal/src/GHC/Internal/Maybe.hs",
        "ext:idx:C:/Users/x/cabal/store/ghc-9.12.1/ghc-internal-9.1401.0/src/GHC/Internal/Base.hs",
    ] {
        assert!(is_framework_ambient_path(&norm(p)), "GHC.Internal prelude should be ambient: {p}");
    }
}

#[test]
fn rejects_cargo_registry_crates() {
    // Third-party cargo deps are `ext:rust:` too but live under `/registry/src/`
    // (or a bare `<crate>/…` reachability path) — they require an explicit `use`
    // and must NOT be ambient.
    for p in [
        "ext:rust:C:/Users/x/.cargo/registry/src/index.crates.io-abc/serde-1.0.0/src/lib.rs",
        "ext:rust:serde/src/de/mod.rs",
    ] {
        assert!(!is_framework_ambient_path(&norm(p)), "cargo dep should NOT be ambient: {p}");
    }
}

#[test]
fn rejects_ordinary_files() {
    for p in [
        "src/app.ts",
        "node_modules/vue/dist/vue.runtime.esm.js", // not .d.ts
        "project/.nuxt/other.ts",                   // wrong basename
        "src/components.d.ts",                      // not under .nuxt
    ] {
        assert!(!is_framework_ambient_path(&norm(p)), "should NOT be ambient: {p}");
    }
}
