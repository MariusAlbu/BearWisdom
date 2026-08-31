// =============================================================================
// ecosystem/ambient_tests.rs — framework ambient path-marker matching.
// =============================================================================

use super::*;
use crate::types::{ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility};

fn norm(p: &str) -> String {
    p.to_lowercase().replace('\\', "/")
}

fn mk_sym(name: &str, qname: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 1,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn pf_with(path: &str, symbols: Vec<ExtractedSymbol>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "rust".to_string(),
        content_hash: "h".to_string(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

#[test]
fn matches_nuxt_and_svelte_and_next_generated() {
    for p in [
        "project/.nuxt/imports.d.ts",
        "project/.nuxt/components.d.ts",
        "app/.svelte-kit/ambient.d.ts",
        "web/.next/next-env.d.ts",
    ] {
        assert!(
            is_framework_ambient_path(&norm(p)),
            "should be ambient: {p}"
        );
    }
}

#[test]
fn matches_root_relative_forms() {
    for p in [
        ".nuxt/imports.d.ts",
        ".svelte-kit/ambient.d.ts",
        ".next/next-env.d.ts",
    ] {
        assert!(
            is_framework_ambient_path(&norm(p)),
            "root-relative should match: {p}"
        );
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
        assert!(
            is_framework_ambient_path(&norm(p)),
            "vue runtime should match: {p}"
        );
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
        assert!(
            is_framework_ambient_path(&norm(p)),
            "bazel builtin should match: {p}"
        );
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
        assert!(
            is_framework_ambient_path(&norm(p)),
            "dart:core should be ambient: {p}"
        );
    }
    // Other dart: libraries need an explicit import — not ambient.
    for p in [
        "ext:idx:C:/x/flutter/cache/dart-sdk/lib/async/stream.dart",
        "ext:idx:C:/x/Pub/Cache/hosted/pub.dev/googleapis-16.0.0/lib/docs/v1.dart",
    ] {
        assert!(
            !is_framework_ambient_path(&norm(p)),
            "non-core dart lib must NOT be ambient: {p}"
        );
    }
}

#[test]
fn matches_haskell_ghc_internal_prelude() {
    for p in [
        "ext:haskell:ghc-internal/src/GHC/Internal/Maybe.hs",
        "ext:idx:C:/Users/x/cabal/store/ghc-9.12.1/ghc-internal-9.1401.0/src/GHC/Internal/Base.hs",
    ] {
        assert!(
            is_framework_ambient_path(&norm(p)),
            "GHC.Internal prelude should be ambient: {p}"
        );
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
        assert!(
            !is_framework_ambient_path(&norm(p)),
            "cargo dep should NOT be ambient: {p}"
        );
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
        assert!(
            !is_framework_ambient_path(&norm(p)),
            "should NOT be ambient: {p}"
        );
    }
}

#[test]
fn lib_path_recognises_ts_lib_types_node_and_stdlib() {
    for p in [
        "ext:ts:__ts_lib__/lib.es5.d.ts",
        "ext:ts:__ts_lib__/lib.dom.d.ts",
        "ext:ts:@types/node/process.d.ts",
        "node_modules/typescript/lib/lib.dom.d.ts",
        "node_modules/@types/node/fs.d.ts",
        "ext:lua-stdlib:string.lua",
        "ext:python-stdlib:os.py",
    ] {
        assert!(is_ambient_global_lib_path(p), "lib source should match: {p}");
    }
}

#[test]
fn lib_path_recognises_framework_ambient_prelude_sources() {
    // Framework-ambient prelude sources are import-free global surfaces: their
    // top-level symbols must enter the ambient scope, same as the ts-lib /
    // <lang>-stdlib sources. The sysroot std subtree lands under an `ext:idx:`
    // tag (not `ext:<lang>-stdlib:`), so the `is_stdlib_external_path` arm alone
    // never classified it.
    for p in [
        "ext:idx:C:/Users/x/.rustup/toolchains/stable-x86_64-pc-windows-msvc/lib/rustlib/src/rust/library/alloc/src/vec/mod.rs",
        "ext:idx:C:/x/flutter/cache/dart-sdk/lib/core/list.dart",
        "ext:idx:C:/Users/x/cabal/store/ghc-9.12.1/ghc-internal-9.1401.0/src/GHC/Internal/Base.hs",
    ] {
        assert!(
            is_ambient_global_lib_path(p),
            "framework-ambient prelude source should be a lib source: {p}"
        );
    }
}

#[test]
fn ambient_qnames_surfaces_prelude_enum_variants() {
    // A prelude source contributes its enum *variants* by their dotted qname so
    // the ambient rung can bind a bare `Some` / `Ok` constructor.
    let prelude = pf_with(
        "ext:idx:C:/u/.rustup/toolchains/stable/lib/rustlib/src/rust/library/core/src/option.rs",
        vec![
            mk_sym("Option", "Option", SymbolKind::Enum),
            mk_sym("Some", "Option.Some", SymbolKind::EnumMember),
            mk_sym("None", "Option.None", SymbolKind::EnumMember),
        ],
    );
    let qn = ambient_global_qnames(&[prelude]);
    assert!(qn.contains("Option"), "top-level enum stays ambient");
    assert!(qn.contains("Option.Some"), "prelude variant Some must be surfaced");
    assert!(qn.contains("Option.None"), "prelude variant None must be surfaced");
}

#[test]
fn ambient_qnames_excludes_variants_from_non_lib_sources() {
    // An ordinary project file's enum variants are not ambient — they need
    // qualification or an explicit import.
    let proj = pf_with(
        "src/app.rs",
        vec![mk_sym("Red", "Color.Red", SymbolKind::EnumMember)],
    );
    let qn = ambient_global_qnames(&[proj]);
    assert!(
        !qn.contains("Color.Red"),
        "project variant must NOT be ambient"
    );
}

#[test]
fn lib_path_rejects_ordinary_externals() {
    for p in [
        "ext:ts:@tanstack/query-core/index.d.ts",
        "src/app.ts",
        "ext:rust:serde/src/lib.rs", // a crate is not -stdlib
    ] {
        assert!(!is_ambient_global_lib_path(p), "must NOT be a lib source: {p}");
    }
}
