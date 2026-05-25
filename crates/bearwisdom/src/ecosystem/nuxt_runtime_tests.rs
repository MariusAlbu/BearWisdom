// =============================================================================
// ecosystem/nuxt_runtime_tests.rs — unit tests for NuxtRuntimeEcosystem.
// =============================================================================

use super::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn returns_empty_when_no_nuxt_dir() {
    let tmp = TempDir::new().unwrap();
    let roots = discover_nuxt_root(tmp.path());
    assert!(roots.is_empty(), "no .nuxt/ → no roots");
}

#[test]
fn returns_root_when_imports_dts_present() {
    let tmp = TempDir::new().unwrap();
    let nuxt_dir = tmp.path().join(".nuxt");
    fs::create_dir_all(&nuxt_dir).unwrap();
    fs::write(
        nuxt_dir.join("imports.d.ts"),
        "export { computed, ref } from 'vue';\n",
    )
    .unwrap();
    let roots = discover_nuxt_root(tmp.path());
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].module_path, "nuxt");
    assert_eq!(roots[0].ecosystem, ECOSYSTEM_TAG);
}

#[test]
fn returns_root_when_only_components_dts_present() {
    let tmp = TempDir::new().unwrap();
    let nuxt_dir = tmp.path().join(".nuxt");
    fs::create_dir_all(&nuxt_dir).unwrap();
    fs::write(
        nuxt_dir.join("components.d.ts"),
        "export const NuxtLink: typeof import('vue').defineComponent;\n",
    )
    .unwrap();
    let roots = discover_nuxt_root(tmp.path());
    assert_eq!(roots.len(), 1);
}

#[test]
fn walk_root_yields_both_dts_files_when_present() {
    let tmp = TempDir::new().unwrap();
    let nuxt_dir = tmp.path().join(".nuxt");
    fs::create_dir_all(&nuxt_dir).unwrap();
    fs::write(nuxt_dir.join("imports.d.ts"), "export {};\n").unwrap();
    fs::write(nuxt_dir.join("components.d.ts"), "export {};\n").unwrap();
    let files = nuxt_auto_import_files(tmp.path());
    assert_eq!(files.len(), 2);
    let names: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert!(names
        .iter()
        .any(|p| p.replace('\\', "/").contains(".nuxt/imports.d.ts")));
    assert!(names
        .iter()
        .any(|p| p.replace('\\', "/").contains(".nuxt/components.d.ts")));
    assert!(files.iter().all(|f| f.language == "typescript"));
}

#[test]
fn walk_root_yields_only_present_files() {
    let tmp = TempDir::new().unwrap();
    let nuxt_dir = tmp.path().join(".nuxt");
    fs::create_dir_all(&nuxt_dir).unwrap();
    fs::write(nuxt_dir.join("imports.d.ts"), "export {};\n").unwrap();
    // components.d.ts intentionally absent
    let files = nuxt_auto_import_files(tmp.path());
    assert_eq!(files.len(), 1);
}
