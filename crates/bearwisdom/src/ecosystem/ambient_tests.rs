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
