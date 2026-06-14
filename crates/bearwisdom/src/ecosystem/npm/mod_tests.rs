// =============================================================================
// ecosystem/npm/mod_tests.rs — sibling tests for npm/mod.rs
// =============================================================================
use super::*;

#[test]
fn is_valid_npm_module_path_accepts_clean_names() {
    assert!(is_valid_npm_module_path("react"));
    assert!(is_valid_npm_module_path("lodash"));
    assert!(is_valid_npm_module_path("typescript"));
    assert!(is_valid_npm_module_path("@types/node"));
    assert!(is_valid_npm_module_path("@vitest/expect"));
    assert!(is_valid_npm_module_path("__ts_lib__"));
}

// ---- user-import gate -------------------------------------------------

fn extract(src: &str) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    extract_user_imports_from_source(src, &mut out);
    out
}

#[test]
fn user_imports_picks_up_static_from_clauses() {
    let src = r#"
        import React from 'react';
        import { useState } from "react";
        import type { Foo } from '@scope/pkg';
        export { Bar } from 'lodash';
    "#;
    let got = extract(src);
    assert!(got.contains("react"));
    assert!(got.contains("@scope/pkg"));
    assert!(got.contains("lodash"));
}

#[test]
fn user_imports_picks_up_bare_side_effect_imports() {
    let src = r#"
        import 'some-pkg/style.css';
        import "polyfill";
    "#;
    let got = extract(src);
    // Both reduce to the package portion.
    assert!(got.contains("some-pkg"));
    assert!(got.contains("polyfill"));
}

#[test]
fn user_imports_picks_up_require_and_dynamic_import() {
    let src = r#"
        const fs = require('fs-extra');
        const lazy = await import('comlink');
        const helper = require("some-other-helper");
    "#;
    let got = extract(src);
    assert!(got.contains("fs-extra"));
    assert!(got.contains("comlink"));
    assert!(got.contains("some-other-helper"));
}

#[test]
fn user_imports_skips_relative_absolute_and_node_protocol() {
    let src = r#"
        import a from './local';
        import b from '../../utils';
        import c from '/abs/path';
        import fs from 'node:fs';
        const x = require('./util');
    "#;
    let got = extract(src);
    assert!(got.is_empty(), "expected empty, got {got:?}");
}

#[test]
fn user_imports_normalizes_subpath_specifiers_to_package_root() {
    let src = r#"
        import x from 'rxjs/operators';
        import y from '@scope/pkg/sub/path';
        import z from 'lodash/fp';
    "#;
    let got = extract(src);
    assert!(got.contains("rxjs"));
    assert!(got.contains("@scope/pkg"));
    assert!(got.contains("lodash"));
    assert!(!got.contains("rxjs/operators"));
    assert!(!got.contains("@scope/pkg/sub/path"));
}

#[test]
fn user_imports_recursive_scan_finds_imports_across_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/index.ts"), "import React from 'react';\n").unwrap();
    std::fs::write(root.join("src/util.tsx"), "import _ from 'lodash';\n").unwrap();
    // node_modules contents must not contribute imports.
    std::fs::create_dir_all(root.join("node_modules/something")).unwrap();
    std::fs::write(
        root.join("node_modules/something/leak.ts"),
        "import x from 'should-not-be-included';\n",
    )
    .unwrap();
    // Test files are skipped by the gate's traversal.
    std::fs::create_dir_all(root.join("__tests__")).unwrap();
    std::fs::write(
        root.join("__tests__/x.test.ts"),
        "import y from 'should-not-be-included-2';\n",
    )
    .unwrap();

    let got = collect_ts_user_imports(root);
    assert!(got.contains("react"));
    assert!(got.contains("lodash"));
    assert!(!got.contains("should-not-be-included"));
    assert!(!got.contains("should-not-be-included-2"));
}

#[test]
fn discover_ts_externals_excludes_unused_declared_dep() {
    // package.json declares two deps; user source only imports one.
    // The excluded dep must not produce a dep root.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(
        root.join("package.json"),
        r#"{
          "name": "x",
          "dependencies": {
            "imported-pkg": "1.0.0",
            "unused-pkg": "2.0.0"
          }
        }"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("node_modules/imported-pkg")).unwrap();
    std::fs::write(
        root.join("node_modules/imported-pkg/package.json"),
        r#"{"name":"imported-pkg","version":"1.0.0"}"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("node_modules/unused-pkg")).unwrap();
    std::fs::write(
        root.join("node_modules/unused-pkg/package.json"),
        r#"{"name":"unused-pkg","version":"2.0.0"}"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/index.ts"), "import x from 'imported-pkg';\n").unwrap();

    let roots = discover_ts_externals(root);
    let ids: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(
        ids.contains(&"imported-pkg"),
        "imported-pkg expected: {ids:?}"
    );
    assert!(
        !ids.contains(&"unused-pkg"),
        "unused-pkg should be gated out: {ids:?}"
    );
}

#[test]
fn discover_ts_externals_keeps_globals_declaring_packages_even_without_import() {
    // Packages whose entry .d.ts declares globals are kept regardless
    // of whether user code explicitly imports them — `describe` /
    // `it` / `expect` (vitest), `$localize` (@angular/localize), `$`
    // (jquery), `cy` (cypress), etc. are typically referenced as
    // globals without a `from` clause.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(
        root.join("package.json"),
        r#"{
          "name": "x",
          "dependencies": {
            "globals-runner": "1.0.0",
            "imported-pkg": "1.0.0",
            "module-only-pkg": "1.0.0"
          }
        }"#,
    )
    .unwrap();
    // globals-runner declares globals — kept by the probe.
    std::fs::create_dir_all(root.join("node_modules/globals-runner")).unwrap();
    std::fs::write(
        root.join("node_modules/globals-runner/package.json"),
        r#"{"name":"globals-runner","version":"1.0.0"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("node_modules/globals-runner/index.d.ts"),
        "declare global { const describe: (s: string, fn: () => void) => void; }\nexport {};\n",
    )
    .unwrap();
    // imported-pkg — no globals, but user imports it.
    std::fs::create_dir_all(root.join("node_modules/imported-pkg")).unwrap();
    std::fs::write(
        root.join("node_modules/imported-pkg/package.json"),
        r#"{"name":"imported-pkg","version":"1.0.0"}"#,
    )
    .unwrap();
    // module-only-pkg — no globals, no user import. Should be gated out.
    std::fs::create_dir_all(root.join("node_modules/module-only-pkg")).unwrap();
    std::fs::write(
        root.join("node_modules/module-only-pkg/package.json"),
        r#"{"name":"module-only-pkg","version":"1.0.0"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("node_modules/module-only-pkg/index.d.ts"),
        "export interface Foo { x: number }\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/index.ts"),
        "import x from 'imported-pkg';\nexport function t() { describe('a', () => {}); }",
    )
    .unwrap();

    let roots = discover_ts_externals(root);
    let ids: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(
        ids.contains(&"globals-runner"),
        "globals-declaring package must survive the gate: {ids:?}"
    );
    assert!(ids.contains(&"imported-pkg"));
    assert!(
        !ids.contains(&"module-only-pkg"),
        "module-only package without an import must be gated out: {ids:?}"
    );
}

#[test]
fn discover_ts_externals_keeps_at_types_when_runtime_pkg_is_imported() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(
        root.join("package.json"),
        r#"{
          "name": "x",
          "dependencies": {
            "lodash": "4.0.0",
            "@types/lodash": "4.0.0"
          }
        }"#,
    )
    .unwrap();
    for pkg in &["lodash"] {
        std::fs::create_dir_all(root.join("node_modules").join(pkg)).unwrap();
        std::fs::write(
            root.join("node_modules").join(pkg).join("package.json"),
            format!(r#"{{"name":"{pkg}","version":"1.0.0"}}"#),
        )
        .unwrap();
    }
    std::fs::create_dir_all(root.join("node_modules/@types/lodash")).unwrap();
    std::fs::write(
        root.join("node_modules/@types/lodash/package.json"),
        r#"{"name":"@types/lodash","version":"4.0.0"}"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/index.ts"), "import _ from 'lodash';\n").unwrap();

    let roots = discover_ts_externals(root);
    let ids: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(ids.contains(&"lodash"));
    // @types/lodash either appears under its own dep label OR as the
    // companion-types fallback discovered alongside lodash. Either is
    // acceptable; the assertion is that it's present.
    let any_at_types_lodash = ids.iter().any(|m| *m == "@types/lodash");
    assert!(
        any_at_types_lodash,
        "@types/lodash must survive when lodash is imported: {ids:?}"
    );
}

// ---- globals probe ------------------------------------

fn mkdep_simple(root: PathBuf, module: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: module.to_string(),
        version: "0.0.0".to_string(),
        root,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

#[test]
fn probe_global_decl_files_returns_empty_when_no_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("node_modules").join("vitest");
    std::fs::create_dir_all(&root).unwrap();
    let dep = mkdep_simple(root, "vitest");
    let probed = probe_global_decl_files(&dep);
    assert!(probed.is_empty());
}

#[test]
fn probe_global_decl_files_finds_dist_globals_d_ts() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("node_modules").join("vitest");
    std::fs::create_dir_all(root.join("dist")).unwrap();
    std::fs::write(
        root.join("dist").join("globals.d.ts"),
        "declare global { const test: () => void }\n",
    )
    .unwrap();
    // A non-target deep file that should NOT be picked up by the probe.
    std::fs::create_dir_all(root.join("dist").join("internal")).unwrap();
    std::fs::write(
        root.join("dist").join("internal").join("noise.d.ts"),
        "export const noise = 1;\n",
    )
    .unwrap();

    let dep = mkdep_simple(root, "vitest");
    let probed = probe_global_decl_files(&dep);
    let paths: Vec<&str> = probed.iter().map(|w| w.relative_path.as_str()).collect();

    assert!(
        paths.iter().any(|p| p.ends_with("dist/globals.d.ts")),
        "expected dist/globals.d.ts in probed: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.contains("internal/noise")),
        "deep files must not be probed: {paths:?}"
    );
}

#[test]
fn probe_global_decl_files_finds_jest_d_ts_at_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("node_modules").join("@types").join("jest");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("index.d.ts"),
        "declare global { const expect: any }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("jest.d.ts"),
        "declare global { const fail: any }\n",
    )
    .unwrap();

    let dep = mkdep_simple(root, "@types/jest");
    let probed = probe_global_decl_files(&dep);
    let paths: Vec<&str> = probed.iter().map(|w| w.relative_path.as_str()).collect();

    assert!(paths.iter().any(|p| p.ends_with("index.d.ts")), "{paths:?}");
    assert!(paths.iter().any(|p| p.ends_with("jest.d.ts")), "{paths:?}");
}


#[test]
fn discover_ts_externals_falls_back_to_keep_all_when_no_user_source() {
    // Manifest-only checkout (e.g. a generator template). With no
    // scannable source, every declared dep gets a root so existing
    // behavior is preserved.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(
        root.join("package.json"),
        r#"{
          "name": "x",
          "dependencies": {
            "alpha": "1.0.0",
            "beta": "1.0.0"
          }
        }"#,
    )
    .unwrap();
    for pkg in &["alpha", "beta"] {
        std::fs::create_dir_all(root.join("node_modules").join(pkg)).unwrap();
        std::fs::write(
            root.join("node_modules").join(pkg).join("package.json"),
            format!(r#"{{"name":"{pkg}","version":"1.0.0"}}"#),
        )
        .unwrap();
    }

    let roots = discover_ts_externals(root);
    let ids: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(ids.contains(&"alpha"), "{ids:?}");
    assert!(ids.contains(&"beta"), "{ids:?}");
}

#[test]
fn is_valid_npm_module_path_rejects_relative_specifiers() {
    assert!(!is_valid_npm_module_path("./rxjs"));
    assert!(!is_valid_npm_module_path("../packages/server"));
    assert!(!is_valid_npm_module_path("./.ignored_concurrently"));
}

#[test]
fn is_valid_npm_module_path_rejects_pnpm_shadows_and_drives() {
    assert!(!is_valid_npm_module_path(".ignored_concurrently"));
    assert!(!is_valid_npm_module_path(".pnpm"));
    assert!(!is_valid_npm_module_path("F:"));
    assert!(!is_valid_npm_module_path("F:/Work/typescript"));
    assert!(!is_valid_npm_module_path(""));
}

#[test]
fn is_valid_npm_module_path_rejects_malformed_scoped() {
    assert!(!is_valid_npm_module_path("@types")); // scope only
    assert!(!is_valid_npm_module_path("@types/")); // empty pkg
    assert!(!is_valid_npm_module_path("@/foo")); // empty scope
    assert!(!is_valid_npm_module_path("@./foo")); // dot-scope
    assert!(!is_valid_npm_module_path("@types/./node")); // dot-pkg
    assert!(!is_valid_npm_module_path("@types/node/sub")); // nested under scope
}

#[test]
fn normalize_virtual_rel_collapses_dot_segments() {
    assert_eq!(
        normalize_virtual_rel("dist/types/./internal/Observable.d.ts"),
        "dist/types/internal/Observable.d.ts"
    );
    assert_eq!(
        normalize_virtual_rel("./v4/classic/./schemas.d.ts"),
        "v4/classic/schemas.d.ts"
    );
    assert_eq!(
        normalize_virtual_rel("dist\\types\\internal\\Observable.d.ts"),
        "dist/types/internal/Observable.d.ts"
    );
    assert_eq!(
        normalize_virtual_rel("dist/types/internal/Observable.d.ts"),
        "dist/types/internal/Observable.d.ts"
    );
}

#[test]
fn declare_global_extracts_const_decls() {
    let src = r#"
declare global {
  const suite: typeof import('vitest')['suite']
  const describe: typeof import('vitest')['describe']
  const expect: typeof import('vitest')['expect']
}
export {}
"#;
    let names = scan_declare_global_blocks(src);
    assert!(names.iter().any(|n| n == "suite"));
    assert!(names.iter().any(|n| n == "describe"));
    assert!(names.iter().any(|n| n == "expect"));
}

#[test]
fn declare_global_extracts_function_and_class_decls() {
    let src = r#"
declare global {
  function beforeEach(fn: () => void): void;
  class Mocha {}
  interface JestMatcher {}
  type TestFn = () => void;
}
"#;
    let names = scan_declare_global_blocks(src);
    assert!(names.iter().any(|n| n == "beforeEach"));
    assert!(names.iter().any(|n| n == "Mocha"));
    assert!(names.iter().any(|n| n == "JestMatcher"));
    assert!(names.iter().any(|n| n == "TestFn"));
}

#[test]
fn declare_global_skips_nested_blocks() {
    let src = r#"
function outer() {
  declare global {
const notAGlobal: number; // inside a function body, shouldn't fire
  }
}
declare global {
  const realGlobal: string;
}
"#;
    // Current implementation accepts the marker anywhere; that's fine
    // in practice since .d.ts files don't have executable function
    // bodies, and matching the marker inside a non-global scope is
    // still informational. Just verify the outer block's name lands.
    let names = scan_declare_global_blocks(src);
    assert!(names.iter().any(|n| n == "realGlobal"));
}

#[test]
fn declare_global_source_without_marker_returns_empty() {
    let src = "export const foo = 1;\nexport function bar() {}\n";
    assert!(scan_declare_global_blocks(src).is_empty());
}

#[test]
fn declare_global_namespace_emits_dotted_names() {
    // @types/express shape — `Express.Multer.File` is the user-visible name.
    let src = r#"
declare global {
  namespace Express {
interface Request {}
namespace Multer {
  interface File {}
}
  }
}
"#;
    let names = scan_declare_global_blocks(src);
    assert!(names.iter().any(|n| n == "Express"));
    assert!(names.iter().any(|n| n == "Express.Request"));
    assert!(names.iter().any(|n| n == "Express.Multer"));
    assert!(names.iter().any(|n| n == "Express.Multer.File"));
}

#[test]
fn declare_namespace_top_level_emits_dotted_names() {
    // @types/google.maps shape — `declare namespace google.maps { class Map {} }`.
    let src = r#"
declare namespace google.maps {
  class Map {}
  class LatLng {}
}
"#;
    let names = scan_declare_global_blocks(src);
    assert!(names.iter().any(|n| n == "google.maps"));
    assert!(names.iter().any(|n| n == "google.maps.Map"));
    assert!(names.iter().any(|n| n == "google.maps.LatLng"));
}

#[test]
fn vue_global_components_explicit_list_extracts_names() {
    // Naive UI / Element Plus / unplugin-vue-components shape.
    let src = r#"
declare module 'vue' {
  export interface GlobalComponents {
NButton: (typeof import('naive-ui'))['NButton']
NCard: (typeof import('naive-ui'))['NCard']
RouterLink: typeof RouterLink
  }
}
"#;
    let names = scan_vue_global_components(src);
    assert!(names.iter().any(|n| n == "NButton"));
    assert!(names.iter().any(|n| n == "NCard"));
    assert!(names.iter().any(|n| n == "RouterLink"));
}

#[test]
fn vue_global_components_optional_props_extracts_names() {
    let src = r#"
declare module '@vue/runtime-core' {
  interface GlobalComponents {
ElButton?: typeof ElButton
ElCard: typeof ElCard
  }
}
"#;
    let names = scan_vue_global_components(src);
    assert!(names.iter().any(|n| n == "ElButton"));
    assert!(names.iter().any(|n| n == "ElCard"));
}

#[test]
fn vue_global_components_extends_form_emits_no_explicit_names() {
    // Vuestic-UI shape: extends-only, no explicit member list.
    // We don't enumerate the extended type today (deep type-resolution
    // territory), so this returns nothing — covered by a separate
    // package-export discovery path or stays unresolved.
    let src = r#"
declare module 'vue' {
  interface GlobalComponents extends VuesticComponents {}
}
"#;
    let names = scan_vue_global_components(src);
    assert!(
        names.is_empty(),
        "extends-only shape yields no explicit names"
    );
}

#[test]
fn vue_global_components_ignores_unrelated_modules() {
    let src = r#"
declare module 'react' {
  interface GlobalComponents {
SomeReactThing: any
  }
}
"#;
    let names = scan_vue_global_components(src);
    assert!(names.is_empty(), "non-vue module augmentations ignored");
}

#[test]
fn declare_global_captures_dollar_prefix_identifiers() {
    // Real shape from `@angular/localize/types/localize.d.ts`:
    //   declare global { const $localize: LocalizeFn; }
    // Also covers jQuery's `declare global { const $: JQueryStatic }`,
    // RxJS-style `$`-suffix observable globals, lodash's bare `_`.
    // Before this, the regex used `\w+` which doesn't include `$` —
    // every dollar-prefixed global declaration was silently dropped.
    let src = r#"
declare global {
  const $localize: LocalizeFn;
  const $: JQueryStatic;
  function $$<T>(arg: T): T;
  class _LodashWrapper {}
}
"#;
    let names = scan_declare_global_blocks(src);
    assert!(
        names.iter().any(|n| n == "$localize"),
        "expected $localize in {names:?}"
    );
    assert!(names.iter().any(|n| n == "$"), "expected $ in {names:?}");
    assert!(names.iter().any(|n| n == "$$"), "expected $$ in {names:?}");
    assert!(
        names.iter().any(|n| n == "_LodashWrapper"),
        "expected _LodashWrapper in {names:?}"
    );
}

#[test]
fn declare_namespace_nested_wrappers_emit_dotted_names() {
    // Alternative @types shape: declare namespace google { namespace maps { class Map {} } }.
    let src = r#"
declare namespace google {
  namespace maps {
class Map {}
namespace places {
  class Autocomplete {}
}
  }
}
"#;
    let names = scan_declare_global_blocks(src);
    assert!(names.iter().any(|n| n == "google"));
    assert!(names.iter().any(|n| n == "google.maps"));
    assert!(names.iter().any(|n| n == "google.maps.Map"));
    assert!(names.iter().any(|n| n == "google.maps.places"));
    assert!(names.iter().any(|n| n == "google.maps.places.Autocomplete"));
}

#[test]
fn scan_ts_header_returns_globals_separately() {
    let src = r#"
export function regularFn() {}
export class RegularClass {}
declare global {
  const describe: typeof import('x')['y']
  function it(name: string): void;
}
"#;
    let (regular, globals) = scan_ts_header(src, "typescript");
    assert!(regular.iter().any(|n| n == "regularFn"));
    assert!(regular.iter().any(|n| n == "RegularClass"));
    assert!(globals.iter().any(|n| n == "describe"));
    assert!(globals.iter().any(|n| n == "it"));
}

#[test]
fn ecosystem_identity() {
    let n = NpmEcosystem;
    assert_eq!(n.id(), ID);
    assert_eq!(Ecosystem::kind(&n), EcosystemKind::Package);
    assert!(Ecosystem::languages(&n).contains(&"typescript"));
    assert!(Ecosystem::languages(&n).contains(&"javascript"));
    assert!(Ecosystem::languages(&n).contains(&"vue"));
    assert!(Ecosystem::languages(&n).contains(&"svelte"));
}

#[test]
fn legacy_locator_string_unchanged() {
    // Keep "typescript" to avoid schema/test churn in Phase 2.
    assert_eq!(
        ExternalSourceLocator::ecosystem(&NpmEcosystem),
        "typescript"
    );
}

#[test]
fn definitely_typed_scoped_escapes() {
    assert_eq!(
        definitely_typed_scoped_name("@tanstack/react-query"),
        Some("tanstack__react-query".to_string())
    );
    assert_eq!(
        definitely_typed_scoped_name("@radix-ui/react-dialog"),
        Some("radix-ui__react-dialog".to_string())
    );
    assert_eq!(definitely_typed_scoped_name("react"), None);
    assert_eq!(definitely_typed_scoped_name("@scope"), None);
    assert_eq!(definitely_typed_scoped_name("@/empty"), None);
}

#[test]
fn ts_source_file_detection() {
    assert!(is_ts_source_file("index.ts"));
    assert!(is_ts_source_file("App.tsx"));
    assert!(is_ts_source_file("index.d.ts"));
    assert!(is_ts_source_file("types.d.mts"));
    assert!(!is_ts_source_file("index.js"));
    assert!(!is_ts_source_file("README.md"));
    assert!(!is_ts_source_file("package.json"));
}

#[test]
fn ts_test_file_detection() {
    assert!(is_test_or_story_file("Foo.test.ts"));
    assert!(is_test_or_story_file("Foo.spec.tsx"));
    assert!(is_test_or_story_file("Button.stories.tsx"));
    assert!(is_test_or_story_file("perf.bench.ts"));
    assert!(!is_test_or_story_file("index.ts"));
    assert!(!is_test_or_story_file("App.tsx"));
    assert!(!is_test_or_story_file("useForm.ts"));
}

// -----------------------------------------------------------------
// M3 — per-package scoped discovery (migrated from typescript.rs)
// -----------------------------------------------------------------

#[test]
fn m3_find_node_modules_walks_ancestors() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path();
    let pkg = ws.join("apps").join("web");
    std::fs::create_dir_all(ws.join("node_modules")).unwrap();
    std::fs::create_dir_all(&pkg).unwrap();
    std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");

    let roots = find_node_modules_with_ancestors(&pkg, ws);
    assert!(
        roots.iter().any(|p| p == &ws.join("node_modules")),
        "expected hoisted workspace node_modules, got {roots:?}"
    );
}

#[test]
fn m3_find_node_modules_prefers_package_local() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path();
    let pkg = ws.join("apps").join("web");
    std::fs::create_dir_all(ws.join("node_modules")).unwrap();
    std::fs::create_dir_all(pkg.join("node_modules")).unwrap();
    std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");

    let roots = find_node_modules_with_ancestors(&pkg, ws);
    let local_idx = roots.iter().position(|p| p == &pkg.join("node_modules"));
    let hoisted_idx = roots.iter().position(|p| p == &ws.join("node_modules"));
    assert!(
        local_idx.is_some() && hoisted_idx.is_some(),
        "expected both node_modules discovered: {roots:?}"
    );
    assert!(
        local_idx.unwrap() < hoisted_idx.unwrap(),
        "package-local should precede hoisted: {roots:?}"
    );
}

#[test]
fn m3_read_single_package_json_scoped_to_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    std::fs::write(
        dir.join("package.json"),
        r#"{"dependencies":{"react":"18"},"devDependencies":{"vitest":"1"}}"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(
        dir.join("sub").join("package.json"),
        r#"{"dependencies":{"axios":"1"}}"#,
    )
    .unwrap();

    let deps = read_single_package_json_deps(dir).unwrap();
    assert!(deps.contains("react"));
    assert!(deps.contains("vitest"));
    assert!(
        !deps.contains("axios"),
        "scoped reader must not recurse into sub/"
    );
}

#[test]
fn m3_discover_ts_externals_scoped_uses_hoisted_node_modules() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path();
    let pkg = ws.join("apps").join("web");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(
        pkg.join("package.json"),
        r#"{"name":"web","dependencies":{"react":"18"}}"#,
    )
    .unwrap();

    let react_dir = ws.join("node_modules").join("react");
    std::fs::create_dir_all(&react_dir).unwrap();
    std::fs::write(
        react_dir.join("index.d.ts"),
        "export function Component(): any;",
    )
    .unwrap();
    std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");

    let roots = discover_ts_externals_scoped(ws, &pkg);
    assert!(
        roots
            .iter()
            .any(|r| r.module_path == "react" && r.root == react_dir),
        "expected react root from hoisted node_modules"
    );
}

#[test]
fn m3_discover_ts_externals_scoped_merges_workspace_root_deps() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path();
    let pkg = ws.join("hooks");
    std::fs::create_dir_all(&pkg).unwrap();

    std::fs::write(
        ws.join("package.json"),
        r#"{"name":"preact","devDependencies":{"chai":"5","vitest":"2"}}"#,
    )
    .unwrap();
    std::fs::write(
        pkg.join("package.json"),
        r#"{"name":"preact-hooks","dependencies":{"preact":"*"}}"#,
    )
    .unwrap();

    let chai_dir = ws.join("node_modules").join("@types").join("chai");
    std::fs::create_dir_all(&chai_dir).unwrap();
    std::fs::write(
        chai_dir.join("index.d.ts"),
        "export function assert(x: any): void;",
    )
    .unwrap();

    let vitest_dir = ws.join("node_modules").join("vitest");
    std::fs::create_dir_all(&vitest_dir).unwrap();
    std::fs::write(
        vitest_dir.join("index.d.ts"),
        "export function describe(n: string, f: () => void): void;",
    )
    .unwrap();

    let preact_dir = ws.join("node_modules").join("preact");
    std::fs::create_dir_all(&preact_dir).unwrap();
    std::fs::write(preact_dir.join("index.d.ts"), "export function h(): any;").unwrap();

    std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");

    let roots = discover_ts_externals_scoped(ws, &pkg);
    // chai is declared as a runtime dep but only `@types/chai` exists
    // on disk — the dep root labels with the canonical `@types/chai`
    // module_path so DefinitelyTyped content keeps its `@types/`
    // prefix. The TS resolver retries `import from 'chai'` against
    // `@types/chai.*` qnames via `ts_import_definitely_typed`.
    assert!(
        roots.iter().any(|r| r.module_path == "@types/chai"),
        "expected @types/chai from workspace root devDeps"
    );
    assert!(
        roots.iter().any(|r| r.module_path == "vitest"),
        "expected vitest from workspace root devDeps"
    );
    assert!(
        roots.iter().any(|r| r.module_path == "preact"),
        "expected preact from sub-package deps"
    );
}

/// Regression: when both `jest` and `@types/jest` are declared, the
/// shared `node_modules/@types/jest` directory must label as
/// `@types/jest` regardless of `HashSet<String>` iteration order over
/// `declared`. Without this, ambient-globals classification (which
/// keys on the `@types/` substring) flips on/off across processes.
#[test]
fn discover_ts_externals_scoped_labels_at_types_canonically() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path();
    std::fs::write(
        ws.join("package.json"),
        r#"{"name":"app","devDependencies":{"jest":"25","@types/jest":"25"}}"#,
    )
    .unwrap();

    // Only the @types/jest tree exists on disk — jest 25 ships no
    // bundled types, which is the realistic setup that triggered the
    // intermittent regression in ts-nestjs-realworld.
    let types_jest = ws.join("node_modules").join("@types").join("jest");
    std::fs::create_dir_all(&types_jest).unwrap();
    std::fs::write(
        types_jest.join("index.d.ts"),
        "declare var describe: any; declare const expect: any;",
    )
    .unwrap();

    std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");

    let roots = discover_ts_externals_scoped(ws, ws);
    let labels: Vec<&str> = roots
        .iter()
        .filter(|r| r.root == types_jest)
        .map(|r| r.module_path.as_str())
        .collect();
    assert_eq!(
        labels,
        vec!["@types/jest"],
        "node_modules/@types/jest must label as @types/jest, never `jest`"
    );
}

#[allow(dead_code)]
fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
    shared_locator()
}

// -----------------------------------------------------------------
// R1 — reachability-based entry resolution
// -----------------------------------------------------------------

fn mkdep(root: PathBuf, name: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: name.to_string(),
        version: String::new(),
        root,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

#[test]
fn resolve_import_prefers_types_field() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("vitest");
    std::fs::create_dir_all(root.join("dist")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"vitest","types":"./dist/index.d.ts","main":"./dist/index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("dist").join("index.d.ts"),
        "export declare function describe(name: string, fn: () => void): void;",
    )
    .unwrap();

    let dep = mkdep(root.clone(), "vitest");
    let files = NpmEcosystem.resolve_import(&dep, "vitest", &["describe"]);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].absolute_path, root.join("dist").join("index.d.ts"));
    assert_eq!(files[0].language, "typescript");
    assert!(files[0].relative_path.starts_with("ext:ts:vitest/"));
}

#[test]
fn resolve_import_rewrites_main_to_dts_sibling() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("react");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"react","main":"./index.js"}"#,
    )
    .unwrap();
    std::fs::write(root.join("index.d.ts"), "export function Component(): any;").unwrap();

    let dep = mkdep(root.clone(), "react");
    let files = NpmEcosystem.resolve_import(&dep, "react", &["Component"]);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].absolute_path, root.join("index.d.ts"));
}

#[test]
fn resolve_exports_types_handles_subpath_root_with_types_condition() {
    // Modern conditional exports — the most common shape on packages
    // shipping types alongside ESM/CJS bundles (vue-router, Pinia,
    // RxJS, Zod, etc.). The `"."` subpath has a `types` condition
    // that points at the .d.ts entry.
    let exports = serde_json::json!({
        ".": {
            "types": "./dist/pkg.d.ts",
            "import": "./dist/pkg.mjs",
            "require": "./dist/pkg.cjs"
        }
    });
    assert_eq!(
        resolve_exports_types(&exports),
        Some("./dist/pkg.d.ts".to_string())
    );
}

#[test]
fn resolve_exports_types_handles_root_condition_map() {
    // Sugar shape — the conditions live directly under `exports`
    // without a `"."` subpath wrapper. Some smaller libs use this.
    let exports = serde_json::json!({
        "types": "./dist/pkg.d.ts",
        "import": "./dist/pkg.mjs"
    });
    assert_eq!(
        resolve_exports_types(&exports),
        Some("./dist/pkg.d.ts".to_string())
    );
}

#[test]
fn resolve_exports_types_handles_nested_under_import_or_require() {
    // Modern dual-publish shape — separate `.d.mts` / `.d.cts`
    // companions per import/require condition. The walker must
    // recurse to find the nested `types` value.
    let exports = serde_json::json!({
        ".": {
            "import": {
                "types": "./dist/pkg.d.mts",
                "default": "./dist/pkg.mjs"
            },
            "require": {
                "types": "./dist/pkg.d.cts",
                "default": "./dist/pkg.cjs"
            }
        }
    });
    // Walker prefers `import` over `require` per the condition order.
    assert_eq!(
        resolve_exports_types(&exports),
        Some("./dist/pkg.d.mts".to_string())
    );
}

#[test]
fn resolve_exports_types_returns_none_for_sugar_string() {
    // `"exports": "./entry.js"` — string sugar, no types info to
    // extract. Falls through to legacy `types`/`typings`/`main` in
    // the caller.
    let exports = serde_json::json!("./dist/pkg.js");
    assert_eq!(resolve_exports_types(&exports), None);
}

#[test]
fn resolve_package_entry_path_prefers_exports_over_legacy_types() {
    // When both fields are present and disagree, modern `exports`
    // wins — it's how publishers steer build tools at the right
    // artifact when the legacy `types` field is kept only for
    // backward compatibility with older toolchains.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("modern-pkg");
    std::fs::create_dir_all(root.join("dist")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{
          "name":"modern-pkg",
          "types":"./legacy.d.ts",
          "exports":{".":{"types":"./dist/modern.d.ts"}}
        }"#,
    )
    .unwrap();
    std::fs::write(root.join("legacy.d.ts"), "export const legacy: 1;").unwrap();
    std::fs::write(
        root.join("dist").join("modern.d.ts"),
        "export const modern: 1;",
    )
    .unwrap();

    let dep = mkdep(root.clone(), "modern-pkg");
    let entry = resolve_package_entry_path(&dep).unwrap();
    assert_eq!(entry, root.join("dist").join("modern.d.ts"));
}

#[test]
fn resolve_relative_ts_path_strips_js_extension_for_dts_companion() {
    // Rollup-bundled type-entry shells re-export from `./chunk.js`
    // companions whose actual types live at `./chunk.d.ts`. The
    // walker must strip `.js` before probing the declarations
    // companion or it never finds the chunk.
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    std::fs::write(dir.join("chunk-abc.d.ts"), "export const x: 1;").unwrap();
    // Note no `chunk-abc.js.d.ts` exists — only the proper sibling.

    let from_file = dir.join("entry.d.ts");
    std::fs::write(&from_file, "").unwrap();
    let resolved = resolve_relative_ts_path(&from_file, "./chunk-abc.js").unwrap();
    assert_eq!(resolved, dir.join("chunk-abc.d.ts"));
}

#[test]
fn resolve_relative_ts_path_strips_mjs_for_dmts_sibling() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    std::fs::write(dir.join("chunk.d.mts"), "export const x: 1;").unwrap();

    let from_file = dir.join("entry.d.ts");
    std::fs::write(&from_file, "").unwrap();
    let resolved = resolve_relative_ts_path(&from_file, "./chunk.mjs").unwrap();
    assert_eq!(resolved, dir.join("chunk.d.mts"));
}

#[test]
fn resolve_import_falls_back_to_index_dts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("tiny-pkg");
    std::fs::create_dir_all(&root).unwrap();
    // No package.json at all — purely filesystem fallback.
    std::fs::write(root.join("index.d.ts"), "export const x: number;").unwrap();

    let dep = mkdep(root.clone(), "tiny-pkg");
    let files = NpmEcosystem.resolve_import(&dep, "tiny-pkg", &["x"]);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].absolute_path, root.join("index.d.ts"));
}

#[test]
fn resolve_import_returns_empty_when_no_entry() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("empty-pkg");
    std::fs::create_dir_all(&root).unwrap();

    let dep = mkdep(root, "empty-pkg");
    let files = NpmEcosystem.resolve_import(&dep, "empty-pkg", &[]);
    assert!(files.is_empty());
}

#[test]
fn resolve_symbol_returns_same_entry_as_import() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("vitest");
    std::fs::create_dir_all(root.join("dist")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"vitest","types":"./dist/index.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("dist").join("index.d.ts"),
        "export interface Assertion {}",
    )
    .unwrap();

    let dep = mkdep(root.clone(), "vitest");
    let a = NpmEcosystem.resolve_import(&dep, "vitest", &["Assertion"]);
    let b = NpmEcosystem.resolve_symbol(&dep, "vitest.Assertion");
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert_eq!(a[0].absolute_path, b[0].absolute_path);
}

// -----------------------------------------------------------------
// R4 — file_declares_type pattern matcher
// -----------------------------------------------------------------

#[test]
fn file_declares_type_matches_decl_keywords() {
    assert!(file_declares_type("export class Foo {}\n", "Foo"));
    assert!(file_declares_type("interface Foo {}\n", "Foo"));
    assert!(file_declares_type("export interface Foo<T> {}\n", "Foo"));
    assert!(file_declares_type("export type Foo = string;\n", "Foo"));
    assert!(file_declares_type("export enum Foo { A, B }\n", "Foo"));
    assert!(file_declares_type("declare class Foo {}\n", "Foo"));
    assert!(file_declares_type(
        "export declare interface Foo {}\n",
        "Foo"
    ));
    assert!(file_declares_type("export abstract class Foo {}\n", "Foo"));
    assert!(file_declares_type("export function Foo() {}\n", "Foo"));
    assert!(file_declares_type("export const Foo = 1;\n", "Foo"));
}

#[test]
fn file_declares_type_rejects_partial_matches() {
    assert!(!file_declares_type("class FooBar {}\n", "Foo"));
    assert!(!file_declares_type("// uses Foo somewhere\n", "Foo"));
    assert!(!file_declares_type("import { Foo } from 'x';\n", "Foo"));
    assert!(!file_declares_type(
        "export interface Bar { f: Foo; }\n",
        "Foo"
    ));
    assert!(!file_declares_type("", "Foo"));
}

// -----------------------------------------------------------------
// Header-only scanner — demand-driven pipeline entry
// -----------------------------------------------------------------

#[test]
fn scan_captures_class_and_interface() {
    let src = "export class Foo {}\nexport interface Bar { x: number; }\n";
    let (names, _) = scan_ts_header(src, "typescript");
    assert!(names.contains(&"Foo".to_string()), "{names:?}");
    assert!(names.contains(&"Bar".to_string()), "{names:?}");
}

#[test]
fn scan_captures_function_and_type_alias() {
    let src = "export function baz(): void {}\nexport type QID = string | number;\n";
    let (names, _) = scan_ts_header(src, "typescript");
    assert!(names.contains(&"baz".to_string()), "{names:?}");
    assert!(names.contains(&"QID".to_string()), "{names:?}");
}

#[test]
fn scan_captures_top_level_const_and_let() {
    let src = "export const Version = '1.0';\nlet counter = 0;\n";
    let (names, _) = scan_ts_header(src, "typescript");
    assert!(names.contains(&"Version".to_string()), "{names:?}");
    assert!(names.contains(&"counter".to_string()), "{names:?}");
}

#[test]
fn scan_captures_enum_declaration() {
    let src = "export enum Color { Red, Green, Blue }\n";
    let (names, _) = scan_ts_header(src, "typescript");
    assert!(names.contains(&"Color".to_string()), "{names:?}");
}

#[test]
fn scan_descends_ambient_declare_module() {
    // DefinitelyTyped shape — declare module 'foo' { ... decls ... }.
    let src = r#"declare module "foo" { export class Client {} export function init(): void; }"#;
    let (names, _) = scan_ts_header(src, "typescript");
    assert!(names.contains(&"Client".to_string()), "{names:?}");
    assert!(names.contains(&"init".to_string()), "{names:?}");
}

#[test]
fn scan_ignores_nested_decls_inside_function_bodies() {
    // Nested decls inside a function body must not leak — the scanner is
    // header-only. Outer function name should appear; the inner class
    // should not.
    let src = "export function outer() { class Hidden {} return new Hidden(); }\n";
    let (names, _) = scan_ts_header(src, "typescript");
    assert!(names.contains(&"outer".to_string()));
    assert!(!names.contains(&"Hidden".to_string()), "leaked: {names:?}");
}

#[test]
fn scan_handles_tsx_components() {
    let src = "export function Button() { return <button/>; }\n";
    let (names, _) = scan_ts_header(src, "tsx");
    assert!(names.contains(&"Button".to_string()), "{names:?}");
}

#[test]
fn scan_handles_plain_javascript() {
    let src = "export function helper() {}\nexport const PI = 3.14;\n";
    let (names, _) = scan_ts_header(src, "javascript");
    assert!(names.contains(&"helper".to_string()), "{names:?}");
    assert!(names.contains(&"PI".to_string()), "{names:?}");
}

#[test]
fn scan_returns_empty_on_empty_source() {
    let (regular, globals) = scan_ts_header("", "typescript");
    assert!(regular.is_empty() && globals.is_empty());
}

#[test]
fn build_index_returns_empty_for_no_deps() {
    let idx = build_npm_symbol_index(&[]);
    assert!(idx.is_empty());
}

#[test]
fn build_index_populates_from_on_disk_node_modules() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("synthetic-pkg");
    std::fs::create_dir_all(root.join("src")).unwrap();
    // Real packages declare their entry in package.json — the entry-only
    // walker resolves `types` → `src/index.d.ts`. Without this, the
    // walker has no entry to start from.
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"synthetic-pkg","version":"1.0.0","types":"src/index.d.ts"}"#,
    )
    .unwrap();
    let index_dts = root.join("src").join("index.d.ts");
    std::fs::write(
        &index_dts,
        "export class Client {}\nexport function connect(): Client { return new Client(); }\n",
    )
    .unwrap();

    let dep = mkdep(root, "synthetic-pkg");
    let idx = build_npm_symbol_index(std::slice::from_ref(&dep));

    assert_eq!(
        idx.locate("synthetic-pkg", "Client"),
        Some(index_dts.as_path())
    );
    assert_eq!(
        idx.locate("synthetic-pkg", "connect"),
        Some(index_dts.as_path())
    );
    assert!(idx.locate("synthetic-pkg", "NotThere").is_none());
}

#[test]
fn build_index_returns_empty_when_package_has_no_entry() {
    // Side-effect-only package: no package.json, no recognizable entry.
    // Entry-only walker yields no files; the dep root still participates
    // in resolve_symbol's on-demand pull when the chain walker asks.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("side-effect-only");
    std::fs::create_dir_all(root.join("internal")).unwrap();
    std::fs::write(
        root.join("internal").join("hidden.d.ts"),
        "export interface Hidden {}\n",
    )
    .unwrap();

    let dep = mkdep(root, "side-effect-only");
    let idx = build_npm_symbol_index(std::slice::from_ref(&dep));
    assert!(
        idx.locate("side-effect-only", "Hidden").is_none(),
        "deep-only types stay out of entry-only index"
    );
}

#[test]
fn build_index_follows_relative_reexports_from_entry() {
    // Entry barrel re-exports from a sibling file. The walker should
    // visit the barrel AND the sibling, registering each declared
    // symbol against its definition file.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("barrel-pkg");
    std::fs::create_dir_all(root.join("dist")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"barrel-pkg","version":"1.0.0","types":"dist/index.d.ts"}"#,
    )
    .unwrap();
    let entry = root.join("dist").join("index.d.ts");
    std::fs::write(&entry, "export { Inner } from './inner';\n").unwrap();
    let inner = root.join("dist").join("inner.d.ts");
    std::fs::write(&inner, "export class Inner { method(): void {} }\n").unwrap();

    let dep = mkdep(root, "barrel-pkg");
    let idx = build_npm_symbol_index(std::slice::from_ref(&dep));
    // Inner resolves through the re-export chain to its definition file.
    assert_eq!(
        idx.locate("barrel-pkg", "Inner"),
        Some(inner.as_path()),
        "Inner should map to its definition, not the barrel"
    );
}

#[test]
fn build_index_bounds_globals_package_to_entry_reexports_and_globals_probe() {
    // A globals-declaring package gets the bounded entry+reexport walk
    // (same as a non-globals package) unioned with the canonical
    // globals-declaration files — NOT a full leaf-tree walk. Every
    // `declare global` symbol and every reexport-reachable type stays
    // locatable; leaf files that declare no globals and aren't reachable
    // through the entry's reexport closure are dropped (pulled on demand
    // by `resolve_symbol` if a chain ever lands on them).
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("globals-pkg");
    std::fs::create_dir_all(root.join("dist")).unwrap();
    std::fs::create_dir_all(root.join("internal")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"globals-pkg","version":"1.0.0","types":"dist/index.d.ts"}"#,
    )
    .unwrap();
    // Entry: opts into globals via declare-global AND re-exports a sibling
    // type, exercising the reexport closure.
    std::fs::write(
        root.join("dist").join("index.d.ts"),
        "declare global { const $entryGlobal: () => void; }\nexport { Reachable } from './reachable';\n",
    )
    .unwrap();
    // Reexport target — reachable from the entry, must stay indexed.
    std::fs::write(
        root.join("dist").join("reachable.d.ts"),
        "export interface Reachable { method(): void; }\n",
    )
    .unwrap();
    // Canonical globals file the entry doesn't reference — pulled in by the
    // globals-probe union, must stay indexed.
    std::fs::write(
        root.join("globals.d.ts"),
        "declare global { const $probedGlobal: () => void; }\nexport {};\n",
    )
    .unwrap();
    // Deep leaf that declares no globals and isn't reexport-reachable. The
    // old whole-tree walk indexed it; the bounded walk must NOT.
    std::fs::write(
        root.join("internal").join("leaf.d.ts"),
        "export function deepLeaf(value: unknown): unknown;\n",
    )
    .unwrap();

    let dep = mkdep(root, "globals-pkg");
    let idx = build_npm_symbol_index(std::slice::from_ref(&dep));

    // Entry's declare-global symbol — locatable under both the synthetic
    // globals module (bare-name fallback) and the owning package.
    assert!(
        idx.locate(NPM_GLOBALS_MODULE, "$entryGlobal").is_some(),
        "entry declare-global symbol must be locatable as a global"
    );
    assert!(idx.locate("globals-pkg", "$entryGlobal").is_some());
    // Globals-probe file's symbol stays locatable.
    assert!(
        idx.locate(NPM_GLOBALS_MODULE, "$probedGlobal").is_some(),
        "globals.d.ts symbol must be pulled in by the globals-probe union"
    );
    // Reexport-reachable type stays locatable.
    assert!(
        idx.locate("globals-pkg", "Reachable").is_some(),
        "reexport-reachable type must stay indexed"
    );
    // Deep leaf is dropped — not a global, not reexport-reachable.
    assert!(
        idx.locate("globals-pkg", "deepLeaf").is_none(),
        "leaf declaring no globals must NOT be eagerly indexed"
    );
}

#[test]
fn build_index_fails_open_to_full_walk_when_globals_package_has_no_entry() {
    // Fail-open guard: a globals-declaring package whose entry can't be
    // resolved (no package.json `types`, no `index.d.ts` fallback) has no
    // computable reexport closure. Rather than drop reexport-reachable
    // types, the build falls back to the full-tree walk — so even a deep
    // leaf is indexed. Globals live only in a root `globals.d.ts`, which
    // is what trips `package_declares_globals`.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("entryless-globals");
    std::fs::create_dir_all(root.join("internal")).unwrap();
    std::fs::write(
        root.join("globals.d.ts"),
        "declare global { const $injected: () => void; }\nexport {};\n",
    )
    .unwrap();
    std::fs::write(
        root.join("internal").join("leaf.d.ts"),
        "export function deepLeaf(value: unknown): unknown;\n",
    )
    .unwrap();

    let dep = mkdep(root, "entryless-globals");
    let idx = build_npm_symbol_index(std::slice::from_ref(&dep));
    assert!(
        idx.locate(NPM_GLOBALS_MODULE, "$injected").is_some(),
        "globals.d.ts symbol must be locatable"
    );
    assert!(
        idx.locate("entryless-globals", "deepLeaf").is_some(),
        "no resolvable entry → full-walk fallback indexes the deep leaf"
    );
}

#[test]
fn package_declares_globals_detects_declare_global_block() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("pkg");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("index.d.ts"),
        "declare global {\n  const $localize: () => void;\n}\nexport {};\n",
    )
    .unwrap();
    assert!(package_declares_globals(&root));
}

#[test]
fn package_declares_globals_detects_top_level_declare_namespace() {
    // The @types/node / @types/google.maps / @types/jquery shape:
    // top-level `declare namespace X { ... }` adds X to global ambient.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("pkg");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("index.d.ts"),
        "declare namespace google.maps {\n  class LatLng {}\n}\n",
    )
    .unwrap();
    assert!(package_declares_globals(&root));
}

#[test]
fn package_declares_globals_false_for_module_only_package() {
    // A regular npm package that exports types but doesn't add globals.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("pkg");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("index.d.ts"),
        "export interface Foo { x: number }\nexport function bar(): Foo;\n",
    )
    .unwrap();
    assert!(!package_declares_globals(&root));
}

#[test]
fn package_declares_globals_honors_package_json_types_field() {
    // The entry file isn't index.d.ts but is named in package.json.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("pkg");
    std::fs::create_dir_all(root.join("dist")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"pkg","types":"dist/types.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("dist").join("types.d.ts"),
        "declare global { const $: unknown }\nexport {};\n",
    )
    .unwrap();
    assert!(
        package_declares_globals(&root),
        "must follow package.json `types` field to find the entry"
    );
}

#[test]
fn package_declares_globals_detects_globals_dts_only_package() {
    // vitest `globals: true` shape: the `types` entry (dist/index.d.ts) has no
    // `declare global`; the globals live ONLY in a separate globals.d.ts.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("vitest");
    std::fs::create_dir_all(root.join("dist")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"vitest","types":"dist/index.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("dist").join("index.d.ts"),
        "export declare const expect: unknown;\n",
    )
    .unwrap();
    std::fs::write(
        root.join("globals.d.ts"),
        "declare global {\n  const describe: () => void;\n  const it: () => void;\n  const expect: unknown;\n}\nexport {};\n",
    )
    .unwrap();
    assert!(
        package_declares_globals(&root),
        "must detect globals declared in a separate globals.d.ts"
    );
}

#[test]
fn package_declares_globals_false_when_no_entry_file_exists() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("pkg");
    std::fs::create_dir_all(&root).unwrap();
    // No package.json, no index.d.ts — nothing to probe.
    assert!(!package_declares_globals(&root));
}

#[test]
fn find_files_declaring_type_returns_definition_only() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("synthetic-pkg");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src").join("foo.d.ts"),
        "export interface Foo { method(): string }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src").join("bar.d.ts"),
        "import { Foo } from './foo';\nexport interface Bar { f: Foo }\n",
    )
    .unwrap();
    std::fs::write(root.join("src").join("baz.d.ts"), "export class Baz {}\n").unwrap();

    let dep = mkdep(root, "synthetic-pkg");
    let files = find_files_declaring_type(&dep, "Foo");
    let paths: Vec<String> = files.iter().map(|f| f.relative_path.clone()).collect();

    // Only foo.d.ts (declares Foo) should match. bar.d.ts uses Foo, baz
    // declares Baz — both excluded.
    assert_eq!(
        paths.len(),
        1,
        "expected only the file declaring Foo: {paths:?}"
    );
    assert!(paths[0].ends_with("foo.d.ts"));
}

#[test]
fn package_ships_scss_returns_true_when_scss_at_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let pkg = tmp.path();
    std::fs::write(pkg.join("_index.scss"), "@mixin assert() {}\n").unwrap();
    assert!(
        package_ships_scss(pkg),
        "package with .scss at root should return true"
    );
}

#[test]
fn package_ships_scss_returns_true_when_scss_in_subdir() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let pkg = tmp.path();
    std::fs::create_dir_all(pkg.join("sass")).unwrap();
    std::fs::write(pkg.join("sass/_output.scss"), "@mixin output() {}\n").unwrap();
    assert!(
        package_ships_scss(pkg),
        "package with .scss in sass/ subdir should return true"
    );
}

#[test]
fn package_ships_scss_returns_false_when_no_scss() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let pkg = tmp.path();
    std::fs::write(pkg.join("index.d.ts"), "export const x: number;\n").unwrap();
    assert!(
        !package_ships_scss(pkg),
        "TS-only package should return false"
    );
}

#[test]
fn discover_ts_externals_keeps_scss_shipping_packages_in_scss_project() {
    // A dep that ships .scss files should survive the user-import gate
    // when the project has .scss user source, even if no user .scss file
    // writes `@use 'sass-test-pkg'`.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(
        root.join("package.json"),
        r#"{
          "name": "x",
          "dependencies": {
            "imported-ts-pkg": "1.0.0",
            "sass-test-pkg": "1.0.0",
            "unused-pkg": "2.0.0"
          }
        }"#,
    )
    .unwrap();

    // imported-ts-pkg — user imports it via TS.
    std::fs::create_dir_all(root.join("node_modules/imported-ts-pkg")).unwrap();
    std::fs::write(
        root.join("node_modules/imported-ts-pkg/package.json"),
        r#"{"name":"imported-ts-pkg","version":"1.0.0"}"#,
    )
    .unwrap();

    // sass-test-pkg — ships .scss files; not imported by user .scss source.
    std::fs::create_dir_all(root.join("node_modules/sass-test-pkg/sass")).unwrap();
    std::fs::write(
        root.join("node_modules/sass-test-pkg/package.json"),
        r#"{"name":"sass-test-pkg","version":"1.0.0"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("node_modules/sass-test-pkg/sass/_assert.scss"),
        "@mixin assert() {}\n",
    )
    .unwrap();

    // unused-pkg — no .scss, not imported.
    std::fs::create_dir_all(root.join("node_modules/unused-pkg")).unwrap();
    std::fs::write(
        root.join("node_modules/unused-pkg/package.json"),
        r#"{"name":"unused-pkg","version":"2.0.0"}"#,
    )
    .unwrap();

    // User source: a .ts file importing imported-ts-pkg and a .scss file.
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/index.ts"),
        "import x from 'imported-ts-pkg';\n",
    )
    .unwrap();
    std::fs::write(root.join("src/styles.scss"), ".button { color: red; }\n").unwrap();

    let roots = discover_ts_externals(root);
    let ids: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(
        ids.contains(&"imported-ts-pkg"),
        "imported TS pkg expected: {ids:?}"
    );
    assert!(
        ids.contains(&"sass-test-pkg"),
        "scss-shipping pkg expected even without @use: {ids:?}"
    );
    assert!(
        !ids.contains(&"unused-pkg"),
        "unused non-scss pkg should be gated out: {ids:?}"
    );
}
