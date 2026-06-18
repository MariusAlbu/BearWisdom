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

/// Write a runtime-only package with NO `types`/`typings`/`exports.types`
/// field and only a `.js` entry — `resolve_package_entry_path` returns None,
/// so the package is reachable-but-typeless. Its declared members live in a
/// `@types/<name>` companion. Returns the package root.
fn write_typeless_pkg(
    node_modules: &std::path::Path,
    name: &str,
    js_body: &str,
) -> std::path::PathBuf {
    let root = node_modules.join(name);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("package.json"),
        format!(r#"{{"name":"{name}","version":"1.0.0","main":"index.js"}}"#),
    )
    .unwrap();
    std::fs::write(root.join("index.js"), js_body).unwrap();
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

// -------------------------------------------------------------------------
// @types companion follow for transitively-reached typeless packages
// -------------------------------------------------------------------------

#[test]
fn transitive_typeless_pkg_drags_in_types_companion() {
    // A directly-imported runner re-exports its assertion container from a
    // sibling runtime package that ships NO `.d.ts` (typeless). Every member
    // it declares lives in `@types/<sibling>`. The transitive walk reaches the
    // typeless runtime package but must ALSO follow the DefinitelyTyped
    // companion so the container interface enters the index.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(
        root.join("package.json"),
        r#"{ "name": "app", "dependencies": { "runner": "1.0.0" } }"#,
    )
    .unwrap();
    let nm = root.join("node_modules");
    // The runner re-exports a value/type from the typeless sibling.
    write_pkg(
        &nm,
        "runner",
        "export { assertOn } from 'core-assert';\n",
    );
    // The sibling is reachable but typeless — only a `.js`, no `types` field.
    write_typeless_pkg(&nm, "core-assert", "module.exports = {};\n");
    // The declared members live in the DefinitelyTyped companion.
    let types_dir = nm.join("@types").join("core-assert");
    std::fs::create_dir_all(&types_dir).unwrap();
    std::fs::write(
        types_dir.join("package.json"),
        r#"{"name":"@types/core-assert","version":"1.0.0","types":"index.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        types_dir.join("index.d.ts"),
        "export interface Assertion { toBe(v: unknown): Assertion; }\nexport function assertOn(v: unknown): Assertion;\n",
    )
    .unwrap();

    std::fs::create_dir_all(root.join("__tests__")).unwrap();
    std::fs::write(
        root.join("__tests__/a.test.ts"),
        "import { assertOn } from 'runner';\n",
    )
    .unwrap();

    let roots = discover_ts_externals(root);
    let ids: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(ids.contains(&"runner"), "{ids:?}");
    assert!(
        ids.contains(&"core-assert"),
        "the typeless runtime sibling must still be reached: {ids:?}"
    );
    assert!(
        ids.contains(&"@types/core-assert"),
        "the DefinitelyTyped companion of a typeless transitive package must be followed: {ids:?}"
    );

    // The companion's container interface is now locatable in the index.
    let idx = build_npm_symbol_index(&roots);
    assert!(
        idx.locate("@types/core-assert", "Assertion").is_some(),
        "companion container interface must be indexed once the @types package is a root"
    );
}

#[test]
fn transitive_scoped_typeless_pkg_drags_in_scoped_types_companion() {
    // Same follow, but the typeless transitive sibling is scoped — its
    // companion sits at `@types/<scope>__<name>` (DefinitelyTyped escaping).
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(
        root.join("package.json"),
        r#"{ "name": "app", "dependencies": { "runner": "1.0.0" } }"#,
    )
    .unwrap();
    let nm = root.join("node_modules");
    write_pkg(
        &nm,
        "runner",
        "export { assertOn } from '@acme/core-assert';\n",
    );
    let scoped_dir = nm.join("@acme").join("core-assert");
    std::fs::create_dir_all(&scoped_dir).unwrap();
    std::fs::write(
        scoped_dir.join("package.json"),
        r#"{"name":"@acme/core-assert","version":"1.0.0","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(scoped_dir.join("index.js"), "module.exports = {};\n").unwrap();
    // Companion under the DefinitelyTyped scope-escaped name.
    let types_dir = nm.join("@types").join("acme__core-assert");
    std::fs::create_dir_all(&types_dir).unwrap();
    std::fs::write(
        types_dir.join("package.json"),
        r#"{"name":"@types/acme__core-assert","version":"1.0.0","types":"index.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        types_dir.join("index.d.ts"),
        "export interface Assertion { toBe(v: unknown): Assertion; }\n",
    )
    .unwrap();

    std::fs::create_dir_all(root.join("__tests__")).unwrap();
    std::fs::write(
        root.join("__tests__/a.test.ts"),
        "import { assertOn } from 'runner';\n",
    )
    .unwrap();

    let roots = discover_ts_externals(root);
    let ids: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(
        ids.contains(&"@types/acme__core-assert"),
        "scoped companion must be followed under the escaped @types name: {ids:?}"
    );
}

#[test]
fn types_companion_is_skipped_when_runtime_is_self_typed() {
    // When the transitive runtime package ships its own `.d.ts`, no companion
    // is needed; the follow simply finds no `@types/<pkg>` dir and adds
    // nothing extra. Asserts the companion follow does not invent roots.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(
        root.join("package.json"),
        r#"{ "name": "app", "dependencies": { "runner": "1.0.0" } }"#,
    )
    .unwrap();
    let nm = root.join("node_modules");
    write_pkg(&nm, "runner", "export { thing } from 'self-typed';\n");
    write_pkg(
        &nm,
        "self-typed",
        "export interface Thing { go(): Thing; }\nexport const thing: Thing;\n",
    );

    std::fs::create_dir_all(root.join("__tests__")).unwrap();
    std::fs::write(
        root.join("__tests__/a.test.ts"),
        "import { thing } from 'runner';\n",
    )
    .unwrap();

    let roots = discover_ts_externals(root);
    let ids: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(ids.contains(&"self-typed"), "{ids:?}");
    assert!(
        !ids.iter().any(|m| m.starts_with("@types/")),
        "no companion root should be invented when the runtime is self-typed: {ids:?}"
    );
}

#[test]
fn definitely_typed_companion_shapes() {
    // Unscoped → `@types/<pkg>`; scoped → `@types/<scope>__<name>`;
    // an `@types/*` spec has no companion of its own.
    assert_eq!(
        definitely_typed_companion("chai"),
        Some(("@types/chai".to_string(), "@types/chai".to_string()))
    );
    assert_eq!(
        definitely_typed_companion("@acme/widget"),
        Some((
            "@types/acme__widget".to_string(),
            "@types/acme__widget".to_string()
        ))
    );
    assert_eq!(definitely_typed_companion("@types/chai"), None);
}

// -------------------------------------------------------------------------
// Multi-hop transitive closure: side-import peer + companion
// -------------------------------------------------------------------------

#[test]
fn transitive_closure_follows_side_import_peer_and_companion_multi_hop() {
    // The matcher-chain closure the live checkout depends on, reduced to its
    // structural shape. A directly-imported runner (`runner`, the vitest
    // analogue) re-exports its assertion containers from a package the
    // project never declares (`runner-expect`, the @vitest/expect analogue).
    // `runner-expect`'s entry pulls a peer (`bdd-assert`, the chai analogue)
    // through an `import * as ns from 'bdd-assert'` line — a bare SIDE import,
    // not a re-export — and declares the matcher members on its OWN
    // containers. The peer is typeless; its declared types live in the
    // `@types/bdd-assert` companion.
    //
    // The walker must, across passes:
    //   (1) collect `runner-expect` from the runner entry's re-export and
    //       add it as a root,
    //   (2) collect `bdd-assert` from runner-expect's `import * as` line and
    //       add it as a root, and
    //   (3) follow the DefinitelyTyped companion of the typeless peer.
    //
    // The member-declaring containers sit in `runner-expect` (reached at the
    // first transitive pass), so once it is a root the matcher containers and
    // the root value become locatable in the index — the data the chain
    // walker needs to advance `expect(x).toBe(y)` from the root value onto the
    // container members.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(
        root.join("package.json"),
        r#"{ "name": "app", "dependencies": { "runner": "1.0.0" } }"#,
    )
    .unwrap();
    let nm = root.join("node_modules");

    // Hop 0: directly-imported runner, re-exporting its containers + root
    // value from the peer it never re-declares.
    write_pkg(
        &nm,
        "runner",
        "export { Assertion, JestAssertion, ExpectStatic, expect } from 'runner-expect';\n",
    );
    // Hop 1: the member-declaring package. Its entry SIDE-imports the peer
    // (no `export ... from`), then declares the matcher containers locally.
    write_pkg(
        &nm,
        "runner-expect",
        "import * as bdd from 'bdd-assert';\n\
         export interface Assertion { toBe(v: unknown): Assertion; }\n\
         export interface JestAssertion { toEqual(v: unknown): void; }\n\
         export interface ExpectStatic { (v: unknown): Assertion; }\n\
         export declare const expect: ExpectStatic;\n",
    );
    // Hop 2: the typeless peer reached via the side import; its declared types
    // live in the DefinitelyTyped companion.
    write_typeless_pkg(&nm, "bdd-assert", "module.exports = {};\n");
    let types_dir = nm.join("@types").join("bdd-assert");
    std::fs::create_dir_all(&types_dir).unwrap();
    std::fs::write(
        types_dir.join("package.json"),
        r#"{"name":"@types/bdd-assert","version":"6.0.0","types":"index.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        types_dir.join("index.d.ts"),
        "export interface BddAssertion { to: BddAssertion; equal(v: unknown): void; }\n",
    )
    .unwrap();

    // The runner is named only from a test file — matcher libs are test-only,
    // exercising the test-dir gate alongside the transitive walk.
    std::fs::create_dir_all(root.join("__tests__")).unwrap();
    std::fs::write(
        root.join("__tests__/a.test.ts"),
        "import { expect } from 'runner';\nexpect(1).toBe(1);\n",
    )
    .unwrap();

    let roots = discover_ts_externals(root);
    let ids: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();

    // Hop 1: the cross-package re-export drags in the member-declaring package.
    assert!(
        ids.contains(&"runner-expect"),
        "the matcher-container package must be reached through the runner re-export: {ids:?}"
    );
    // Hop 2: the peer reached via `import * as` (a side import, not a
    // re-export) must be followed.
    assert!(
        ids.contains(&"bdd-assert"),
        "the peer reached via `import * as` must be followed multi-hop: {ids:?}"
    );
    // Hop 2 companion: the typeless peer's DefinitelyTyped types are followed.
    assert!(
        ids.contains(&"@types/bdd-assert"),
        "the typeless peer's @types companion must be followed at the same hop: {ids:?}"
    );

    // The matcher containers declared in the reached member package are now
    // locatable in the index.
    let idx = build_npm_symbol_index(&roots);
    for name in ["Assertion", "JestAssertion", "ExpectStatic", "expect"] {
        assert!(
            idx.locate("runner-expect", name).is_some(),
            "`{name}` must be indexed once runner-expect is a root: {ids:?}"
        );
    }
    // The companion's container is locatable under the @types module.
    assert!(
        idx.locate("@types/bdd-assert", "BddAssertion").is_some(),
        "the companion container must be indexed once @types/bdd-assert is a root: {ids:?}"
    );
}

// -------------------------------------------------------------------------
// pnpm store-sibling canonicalisation (symlink-dependent; best-effort)
// -------------------------------------------------------------------------

#[test]
fn transitive_reach_canonicalises_pnpm_store_symlink_to_find_sibling() {
    // pnpm exposes a flat import view over a content-addressed store by
    // symlink: the consumer-visible `node_modules/<dep>` points at the real
    // package in `node_modules/.pnpm/<store_node>/node_modules/<dep>`, where
    // the dep's own transitive packages sit as siblings. `dep_local_node_modules`
    // canonicalises the symlink so the transitive walker finds those siblings.
    //
    // The reach itself is covered portably by
    // `transitive_closure_follows_side_import_peer_and_companion_multi_hop`;
    // this test pins the symlink-canonicalisation hop specifically. Symlink
    // creation needs elevation on Windows, so on a denial the fixture can't be
    // built and the test returns early rather than failing — the contract it
    // pins still holds wherever symlinks are creatable (Unix, elevated Windows).
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(
        root.join("package.json"),
        r#"{ "name": "app", "dependencies": { "runner": "1.0.0" } }"#,
    )
    .unwrap();

    let pnpm = root.join("node_modules").join(".pnpm");
    // The runner's real dir lives in its own store node; its peer is a real
    // sibling in the SAME store node — reachable only by canonicalising the
    // consumer symlink to this store node.
    let runner_store = pnpm.join("runner@1.0.0").join("node_modules");
    let runner_real = write_pkg(
        &runner_store,
        "runner",
        "export { Assertion } from 'runner-expect';\n",
    );
    write_pkg(
        &runner_store,
        "runner-expect",
        "export interface Assertion { toBe(v: unknown): Assertion; }\n",
    );

    // Consumer-visible symlink at top-level node_modules → store real dir.
    let link = root.join("node_modules").join("runner");
    if create_dir_symlink(&runner_real, &link).is_err() {
        eprintln!(
            "skipping pnpm-symlink canonicalisation test: directory symlink \
             creation denied in this environment"
        );
        return;
    }

    std::fs::create_dir_all(root.join("__tests__")).unwrap();
    std::fs::write(
        root.join("__tests__/a.test.ts"),
        "import { expect } from 'runner';\n",
    )
    .unwrap();

    let roots = discover_ts_externals(root);
    let ids: Vec<&str> = roots.iter().map(|r| r.module_path.as_str()).collect();
    assert!(
        ids.contains(&"runner-expect"),
        "the peer must be found as a store sibling after canonicalising the \
         consumer symlink: {ids:?}"
    );
    let idx = build_npm_symbol_index(&roots);
    assert!(
        idx.locate("runner-expect", "Assertion").is_some(),
        "the peer's container must be indexed once reached through the store: {ids:?}"
    );
}

/// Create a directory symlink at `link` pointing at `target`. Returns the
/// platform error (e.g. permission denied) so the caller can skip rather
/// than panic when symlink creation is unavailable.
#[cfg(unix)]
fn create_dir_symlink(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_dir_symlink(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}
