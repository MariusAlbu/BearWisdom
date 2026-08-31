// =============================================================================
// ecosystem/npm/symbol_index_tests.rs — (module, name) → file index
//
// Covers the same-package deep-specifier re-export shape: a package's
// subpath entry re-exports a name from a NON-relative specifier that names
// the package's own bare name plus an internal path (`pkg/dist/inner`, the
// `next/server.d.ts` -> `next/dist/server/.../response.d.ts` shape) — Node's
// self-reference resolution, not a foreign package. Distinguished from the
// genuinely cross-package case, which must keep declining.
// =============================================================================

use super::*;
use crate::ecosystem::externals::ExternalDepRoot;
use crate::ecosystem::npm::LEGACY_ECOSYSTEM_TAG;
use std::path::{Path, PathBuf};

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
fn same_package_deep_path_strips_bare_name_prefix() {
    assert_eq!(
        same_package_deep_path("next/dist/server/response", "next"),
        Some("dist/server/response")
    );
}

#[test]
fn same_package_deep_path_declines_a_different_package() {
    // `next-auth/dist/x` does not name the `next` package — a longer sibling
    // name sharing a prefix must not be mistaken for a deep specifier.
    assert_eq!(same_package_deep_path("next-auth/dist/x", "next"), None);
}

#[test]
fn same_package_deep_path_declines_the_bare_name_alone() {
    // No `/` remainder — this is a plain cross-package bare import, not a
    // deep specifier into a subpath.
    assert_eq!(same_package_deep_path("next", "next"), None);
}

#[test]
fn build_index_follows_same_package_deep_reexport() {
    // The `next/server.d.ts` shape: a subpath entry re-exports a name from a
    // specifier naming the package's OWN bare name plus an internal path —
    // not a relative import, not a foreign package. `NextResponse` lives in
    // the deep file the entry never resolves relatively.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("framework-pkg");
    std::fs::create_dir_all(root.join("dist/server/web/spec")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"framework-pkg","version":"1.0.0","types":"server.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("server.d.ts"),
        "export { FrameworkResponse } from 'framework-pkg/dist/server/web/spec/response';\n",
    )
    .unwrap();
    let deep_file = root.join("dist/server/web/spec/response.d.ts");
    std::fs::write(
        &deep_file,
        "export declare class FrameworkResponse { static json(): FrameworkResponse; }\n",
    )
    .unwrap();

    let dep = mkdep(root, "framework-pkg");
    let idx = build_npm_symbol_index(std::slice::from_ref(&dep));

    assert_eq!(
        idx.locate("framework-pkg", "FrameworkResponse"),
        Some(deep_file.as_path()),
        "must resolve to the deep file that declares the class, not the barrel re-export"
    );
}

#[test]
fn build_index_follows_same_package_deep_reexport_through_a_directory_index_barrel() {
    // The `next/dist/server/after` shape: the deep specifier resolves (via the
    // directory-index fallback) to a barrel `after/index.d.ts` that only does
    // `export * from './after'` — the function itself lives in the sibling
    // `after/after.d.ts`. Landing on the barrel and stopping there (treating
    // it as though it were the declaring file) loses the function entirely;
    // the wildcard must be followed one more hop on disk.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("framework-pkg");
    std::fs::create_dir_all(root.join("dist/server/task")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"framework-pkg","version":"1.0.0","types":"server.d.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("server.d.ts"),
        "export { runTask } from 'framework-pkg/dist/server/task';\n",
    )
    .unwrap();
    std::fs::write(
        root.join("dist/server/task/index.d.ts"),
        "export * from './task';\n",
    )
    .unwrap();
    let declaring_file = root.join("dist/server/task/task.d.ts");
    std::fs::write(&declaring_file, "export declare function runTask(): void;\n").unwrap();

    let dep = mkdep(root, "framework-pkg");
    let idx = build_npm_symbol_index(std::slice::from_ref(&dep));

    assert_eq!(
        idx.locate("framework-pkg", "runTask"),
        Some(declaring_file.as_path()),
        "must follow the directory-index barrel's wildcard to the declaring file"
    );
}

#[test]
fn build_index_still_declines_a_genuinely_cross_package_reexport() {
    // A specifier naming a DIFFERENT package (not this package's bare name)
    // must keep falling back to indexing the name at the barrel file — the
    // other package's own scan owns its Locals.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("wrapper-pkg");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"wrapper-pkg","version":"1.0.0","types":"index.d.ts"}"#,
    )
    .unwrap();
    let entry = root.join("index.d.ts");
    std::fs::write(&entry, "export { Shared } from 'other-pkg';\n").unwrap();

    let dep = mkdep(root, "wrapper-pkg");
    let idx = build_npm_symbol_index(std::slice::from_ref(&dep));

    assert_eq!(
        idx.locate("wrapper-pkg", "Shared"),
        Some(entry.as_path()),
        "cross-package reexport falls back to the barrel, unchanged from before"
    );
    assert_eq!(
        idx.reexport_aliases().count(),
        0,
        "a target package that is not a scanned dep root records no alias"
    );
}

/// Write a minimal package at `node_modules/<dir>` with a `types` entry and
/// the given entry-file source, returning the package root.
fn write_pkg(nm: &Path, dir: &str, name: &str, entry_source: &str) -> PathBuf {
    let root = nm.join(dir);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("package.json"),
        format!(r#"{{"name":"{name}","version":"1.0.0","types":"index.d.ts"}}"#),
    )
    .unwrap();
    std::fs::write(root.join("index.d.ts"), entry_source).unwrap();
    root
}

#[test]
fn cross_package_reexport_records_alias_and_keeps_barrel_location() {
    // A barrel binding a name declared in a SIBLING dep root: the located
    // file stays the barrel (materialized symbols are prefixed by their own
    // file's package), while the bridge is recorded as an alias pointing at
    // the sibling's declaring file under its declared name.
    let tmp = tempfile::TempDir::new().unwrap();
    let nm = tmp.path().join("node_modules");
    let wrapper = write_pkg(&nm, "wrapper-pkg", "wrapper-pkg", "export { Shared } from 'lib-pkg';\n");
    let lib = write_pkg(&nm, "lib-pkg", "lib-pkg", "export declare class Shared { run(): void; }\n");

    let deps = vec![mkdep(wrapper.clone(), "wrapper-pkg"), mkdep(lib.clone(), "lib-pkg")];
    let idx = build_npm_symbol_index(&deps);

    assert_eq!(
        idx.locate("wrapper-pkg", "Shared"),
        Some(wrapper.join("index.d.ts").as_path()),
        "the (module, name) slot must keep pointing at the barrel"
    );
    let aliases: Vec<_> = idx.reexport_aliases().collect();
    assert_eq!(aliases.len(), 1);
    let (module, name, target_file, target_name) = aliases[0];
    assert_eq!(module, "wrapper-pkg");
    assert_eq!(name, "Shared");
    assert_eq!(target_file, lib.join("index.d.ts").as_path());
    assert_eq!(target_name, "Shared");
}

#[test]
fn cross_package_reexport_records_alias_from_a_scoped_sibling() {
    // The same bridge through a scoped package name (`@scope/runner`).
    let tmp = tempfile::TempDir::new().unwrap();
    let nm = tmp.path().join("node_modules");
    let runner_barrel = write_pkg(
        &nm,
        "test-runner",
        "test-runner",
        "export { describeSuite } from '@scope/runner';\n",
    );
    std::fs::create_dir_all(nm.join("@scope")).unwrap();
    let runner = write_pkg(
        &nm,
        "@scope/runner",
        "@scope/runner",
        "export declare const describeSuite: SuiteApi;\ninterface SuiteApi { runIf(c: boolean): void; }\n",
    );

    let deps = vec![
        mkdep(runner_barrel.clone(), "test-runner"),
        mkdep(runner.clone(), "@scope/runner"),
    ];
    let idx = build_npm_symbol_index(&deps);

    assert_eq!(
        idx.locate("test-runner", "describeSuite"),
        Some(runner_barrel.join("index.d.ts").as_path()),
    );
    let aliases: Vec<_> = idx.reexport_aliases().collect();
    assert_eq!(aliases.len(), 1);
    let (module, name, target_file, target_name) = aliases[0];
    assert_eq!(module, "test-runner");
    assert_eq!(name, "describeSuite");
    assert_eq!(target_file, runner.join("index.d.ts").as_path());
    assert_eq!(target_name, "describeSuite");
}

#[test]
fn cross_package_reexport_alias_tracks_a_rename() {
    // `export { Orig as Exposed } from 'lib-pkg'` — the alias is recorded
    // under the EXPOSED name the importing module binds, pointing at the
    // declaration's own name in the sibling package.
    let tmp = tempfile::TempDir::new().unwrap();
    let nm = tmp.path().join("node_modules");
    let wrapper = write_pkg(
        &nm,
        "wrapper-pkg",
        "wrapper-pkg",
        "export { Orig as Exposed } from 'lib-pkg';\n",
    );
    let lib = write_pkg(&nm, "lib-pkg", "lib-pkg", "export declare class Orig {}\n");

    let deps = vec![mkdep(wrapper, "wrapper-pkg"), mkdep(lib.clone(), "lib-pkg")];
    let idx = build_npm_symbol_index(&deps);

    let aliases: Vec<_> = idx.reexport_aliases().collect();
    assert_eq!(aliases.len(), 1);
    let (module, name, target_file, target_name) = aliases[0];
    assert_eq!(module, "wrapper-pkg");
    assert_eq!(name, "Exposed");
    assert_eq!(target_file, lib.join("index.d.ts").as_path());
    assert_eq!(target_name, "Orig");
}

#[test]
fn ambient_declare_module_names_resolve_under_the_declared_key() {
    // A `.d.ts` entry declaring `declare module 'scheme:thing'` registers the
    // declared literal as its own module key: the specifier users import is
    // that literal, not the declaring package's name. Inner names locate
    // under the declared key and the module entry points at the declaring
    // file so demand can materialize it.
    let tmp = tempfile::TempDir::new().unwrap();
    let nm = tmp.path().join("node_modules");
    let root = write_pkg(
        &nm,
        "types-pkg",
        "types-pkg",
        "declare module 'scheme:thing' {\n  export function inner(): void;\n}\n",
    );
    let entry = root.join("index.d.ts");

    let dep = mkdep(root, "types-pkg");
    let idx = build_npm_symbol_index(std::slice::from_ref(&dep));

    assert_eq!(
        idx.locate("scheme:thing", "inner"),
        Some(entry.as_path()),
        "inner exported names must locate under the declared module key"
    );
    assert_eq!(
        idx.module_entry("scheme:thing"),
        Some(entry.as_path()),
        "the declared name must get a module-entry key at the declaring file"
    );
}

#[test]
fn subpath_export_entry_gets_a_module_entry_key() {
    // A package publishing `./test` in its `exports` map (the
    // `playwright/test` shape) must key the full deep specifier so a ref
    // tagged with it materializes the subpath's own entry file.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("node_modules").join("runner-pkg");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"runner-pkg","version":"1.0.0","types":"index.d.ts","exports":{".":{"types":"./index.d.ts"},"./test":{"types":"./test.d.ts"}}}"#,
    )
    .unwrap();
    std::fs::write(root.join("index.d.ts"), "export declare const version: string;\n").unwrap();
    let sub_entry = root.join("test.d.ts");
    std::fs::write(&sub_entry, "export declare function test(name: string): void;\n").unwrap();

    let dep = mkdep(root.clone(), "runner-pkg");
    let idx = build_npm_symbol_index(std::slice::from_ref(&dep));

    assert_eq!(
        idx.module_entry("runner-pkg/test"),
        Some(sub_entry.as_path()),
        "the subpath specifier must key the subpath's entry file"
    );
    assert_eq!(
        idx.module_entry("runner-pkg"),
        Some(root.join("index.d.ts").as_path()),
        "the `.` entry key stays untouched"
    );
}
