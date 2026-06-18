// =============================================================================
// ecosystem/npm/externals_tests.rs — sibling tests for the TS externals pipeline
//
// Exercises the externals discovery + symbol-index surface that lets the
// GENERIC chain walker resolve member calls into external libraries. The
// matcher / DOM-query / assertion containers used here are SYNTHETIC packages
// laid out on disk exactly like a real npm install; nothing keys on a
// framework name. The contract under test is purely structural:
//
//   * a package imported only from test files passes the user-import gate and
//     becomes a dep root,
//   * a cross-package re-export from such a package drags the re-exported
//     package in transitively, and
//   * once a package is a dep root, the symbol index exposes its container
//     interfaces and root values, and the TS extractor emits each container's
//     member rows plus a return type — the data the chain walker needs for a
//     first hop and every member step after it.
// =============================================================================

use super::*;

use crate::ecosystem::npm::build_npm_symbol_index;
use crate::languages::typescript::extract;
use crate::types::SymbolKind;

fn mkdep(root: std::path::PathBuf, module: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: module.to_string(),
        version: "0.0.0".to_string(),
        root,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

/// Write a minimal package: `package.json` naming a `.d.ts` entry plus the
/// entry file body. Returns the package root.
fn write_pkg(node_modules: &std::path::Path, name: &str, entry_body: &str) -> std::path::PathBuf {
    let root = node_modules.join(name);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("package.json"),
        format!(r#"{{"name":"{name}","version":"1.0.0","types":"index.d.ts"}}"#),
    )
    .unwrap();
    std::fs::write(root.join("index.d.ts"), entry_body).unwrap();
    root
}

// -------------------------------------------------------------------------
// Scan-scope: test-file-only imports become dep roots
// -------------------------------------------------------------------------

#[test]
fn discover_keeps_dep_imported_only_from_test_files() {
    // The assertion library is imported ONLY from a test file under
    // `__tests__/`. Before the scan-scope fix the gate pruned that directory,
    // so the dep never became a root and its members stayed absent. It must
    // now survive the gate.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(
        root.join("package.json"),
        r#"{
          "name": "app",
          "dependencies": {
            "runtime-pkg": "1.0.0",
            "assert-lib": "1.0.0"
          }
        }"#,
    )
    .unwrap();
    write_pkg(
        &root.join("node_modules"),
        "runtime-pkg",
        "export const x: number;\n",
    );
    write_pkg(
        &root.join("node_modules"),
        "assert-lib",
        "export interface Assertion { toBe(v: unknown): Assertion; }\nexport function expect(v: unknown): Assertion;\n",
    );

    // Production source imports the runtime package only.
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/index.ts"), "import { x } from 'runtime-pkg';\n").unwrap();
    // The assertion library is named only in a test file.
    std::fs::create_dir_all(root.join("__tests__")).unwrap();
    std::fs::write(
        root.join("__tests__/index.test.ts"),
        "import { expect } from 'assert-lib';\nexpect(1).toBe(1);\n",
    )
    .unwrap();

    let roots = discover_ts_externals(root);
    let ids: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(ids.contains(&"runtime-pkg"), "{ids:?}");
    assert!(
        ids.contains(&"assert-lib"),
        "a dep imported only from a test file must pass the gate: {ids:?}"
    );
}

// -------------------------------------------------------------------------
// Transitive walk reaches a re-exported member package
// -------------------------------------------------------------------------

#[test]
fn discover_walks_into_cross_package_reexport_from_test_only_dep() {
    // The test-file-only dep (`runner`) re-exports its assertion container
    // from a sibling package (`runner-expect`) that the project never declares
    // directly. The transitive re-export walk must pull `runner-expect` in so
    // its members land in the index.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(
        root.join("package.json"),
        r#"{
          "name": "app",
          "dependencies": { "runner": "1.0.0" }
        }"#,
    )
    .unwrap();
    let nm = root.join("node_modules");
    // `runner`'s entry re-exports the assertion container from a sibling pkg.
    write_pkg(
        &nm,
        "runner",
        "export { Assertion, expect } from 'runner-expect';\n",
    );
    // The sibling package — never declared in package.json, only reachable
    // through `runner`'s cross-package re-export.
    write_pkg(
        &nm,
        "runner-expect",
        "export interface Assertion { toBe(v: unknown): Assertion; }\nexport declare function expect(v: unknown): Assertion;\n",
    );

    std::fs::create_dir_all(root.join("__tests__")).unwrap();
    std::fs::write(
        root.join("__tests__/a.test.ts"),
        "import { expect } from 'runner';\n",
    )
    .unwrap();

    let roots = discover_ts_externals(root);
    let ids: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(ids.contains(&"runner"), "{ids:?}");
    assert!(
        ids.contains(&"runner-expect"),
        "transitive re-export must drag in the member package: {ids:?}"
    );

    // And its container interface + root function are locatable in the index.
    let idx = build_npm_symbol_index(&roots);
    assert!(
        idx.locate("runner-expect", "Assertion").is_some(),
        "assertion container interface must be indexed once its package is a root"
    );
    assert!(
        idx.locate("runner-expect", "expect").is_some(),
        "root function must be indexed once its package is a root"
    );
}

// -------------------------------------------------------------------------
// Member rows + return types are emitted from matcher-style .d.ts shapes
// -------------------------------------------------------------------------

#[test]
fn extractor_emits_member_rows_for_assertion_container() {
    // A matcher container interface whose methods return the container itself
    // (the fluent-assertion shape). The chain walker resolves `expect(x).toBe`
    // by finding `toBe` as a member of the container and advancing on its
    // return type — both must be present in the extracted symbols.
    let src = "export interface Assertion {\n  toBe(value: unknown): Assertion;\n  toEqual(value: unknown): Assertion;\n}\n";
    let result = extract::extract(src, false);

    let container = result
        .symbols
        .iter()
        .position(|s| s.name == "Assertion" && s.kind == SymbolKind::Interface)
        .expect("container interface must be extracted");

    let to_be = result
        .symbols
        .iter()
        .find(|s| s.name == "toBe" && s.kind == SymbolKind::Method)
        .expect("toBe must be extracted as a method");
    assert_eq!(
        to_be.parent_index,
        Some(container),
        "toBe must be a member row of the container interface"
    );
    // The member's declared return type is exposed as a TypeRef the resolve
    // pass interns into the member's yield type, so the chain walker can
    // advance from `expect(x).toBe(...)` onto the container again.
    let has_return_typeref = result.refs.iter().any(|r| {
        r.kind == crate::types::EdgeKind::TypeRef && r.target_name == "Assertion"
    });
    assert!(
        has_return_typeref,
        "the member's declared return type must be exposed as a TypeRef: {:?}",
        result.refs
    );
}

#[test]
fn extractor_emits_member_rows_for_object_literal_supertype() {
    // The real DOM-query container composes its query members through a
    // supertype whose body is an object literal of method signatures
    // (`getByText`, `getByRole`). The extractor must emit those as member rows
    // under the container so a chain rooted on a receiver of the container type
    // resolves the query member by the generic member walk + supertype climb.
    let src = "export interface QueryContainer {\n  getByText(text: string): HTMLElement;\n  getByRole(role: string): HTMLElement;\n}\nexport interface RenderResult extends QueryContainer {\n  container: HTMLElement;\n}\n";
    let result = extract::extract(src, false);

    let qc = result
        .symbols
        .iter()
        .position(|s| s.name == "QueryContainer" && s.kind == SymbolKind::Interface)
        .expect("query container must be extracted");
    let get_by_text = result
        .symbols
        .iter()
        .find(|s| s.name == "getByText" && s.kind == SymbolKind::Method)
        .expect("getByText must be extracted as a method");
    assert_eq!(
        get_by_text.parent_index,
        Some(qc),
        "getByText must be a member row of the query container"
    );
    let has_query_return = result.refs.iter().any(|r| {
        r.kind == crate::types::EdgeKind::TypeRef && r.target_name == "HTMLElement"
    });
    assert!(
        has_query_return,
        "query member declared return type must be exposed as a TypeRef: {:?}",
        result.refs
    );

    // The composing container declares the supertype via an Inherits ref, so
    // the inherits map carries the edge the chain walker climbs to reach the
    // query members.
    let has_inherits = result.refs.iter().any(|r| {
        r.kind == crate::types::EdgeKind::Inherits && r.target_name == "QueryContainer"
    });
    assert!(
        has_inherits,
        "RenderResult must emit an Inherits ref to its query supertype: {:?}",
        result.refs
    );
}

#[test]
fn extractor_records_root_function_return_type() {
    // `expect()` / `expectTypeOf()` are the roots a matcher chain starts from.
    // Their declarations must record a return type so the chain walker has a
    // first hop into the matcher container.
    let src = "export interface ExpectTypeOf { toEqualTypeOf(v: unknown): void; }\nexport declare function expectTypeOf(value: unknown): ExpectTypeOf;\n";
    let result = extract::extract(src, false);

    let _root_fn = result
        .symbols
        .iter()
        .find(|s| s.name == "expectTypeOf" && s.kind == SymbolKind::Function)
        .expect("root function must be extracted");
    // The root's declared return type is exposed as a TypeRef the resolve pass
    // interns into the function's return-type id, giving the chain walker its
    // first hop into the matcher container.
    let has_root_return = result.refs.iter().any(|r| {
        r.kind == crate::types::EdgeKind::TypeRef && r.target_name == "ExpectTypeOf"
    });
    assert!(
        has_root_return,
        "root function declared return type must be exposed as a TypeRef: {:?}",
        result.refs
    );
}
