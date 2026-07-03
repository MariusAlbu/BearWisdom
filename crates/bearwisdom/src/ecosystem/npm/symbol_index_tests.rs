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
use std::path::PathBuf;

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
}
