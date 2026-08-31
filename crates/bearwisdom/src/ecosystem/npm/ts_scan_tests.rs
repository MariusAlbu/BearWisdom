// =============================================================================
// ecosystem/npm/ts_scan_tests.rs — header-scan export/ambient-module capture
// =============================================================================

use super::*;

#[test]
fn two_declare_module_blocks_yield_both_names_with_inner_exports() {
    let source = r#"
declare module 'virtual:pwa-register' {
  export interface RegisterSWOptions {
    immediate?: boolean;
  }
  export function registerSW(options?: RegisterSWOptions): () => Promise<void>;
}

declare module 'astro:content' {
  export function getCollection(name: string): Promise<unknown[]>;
  export class CollectionEntry {}
}
"#;
    let exports = scan_ts_file_exports(source, "typescript");
    assert_eq!(exports.ambient_modules.len(), 2, "both declared names must surface");

    let (name_a, inner_a) = &exports.ambient_modules[0];
    assert_eq!(name_a, "virtual:pwa-register");
    assert_eq!(
        inner_a,
        &vec!["RegisterSWOptions".to_string(), "registerSW".to_string()],
        "inner exported names, sorted"
    );

    let (name_b, inner_b) = &exports.ambient_modules[1];
    assert_eq!(name_b, "astro:content");
    assert_eq!(
        inner_b,
        &vec!["CollectionEntry".to_string(), "getCollection".to_string()],
    );
}

#[test]
fn shorthand_declaration_yields_the_name_with_no_inner_exports() {
    let source = "declare module 'my-untyped-shim';\n";
    let exports = scan_ts_file_exports(source, "typescript");
    assert_eq!(
        exports.ambient_modules,
        vec![("my-untyped-shim".to_string(), Vec::new())],
    );
}

#[test]
fn plain_files_yield_empty_ambient_modules() {
    let source = r#"
export const answer = 42;
export function compute(): number { return answer; }
declare namespace Config {
  const timeout: number;
}
"#;
    let exports = scan_ts_file_exports(source, "typescript");
    assert!(
        exports.ambient_modules.is_empty(),
        "no string-named ambient module declared; got {:?}",
        exports.ambient_modules
    );
}
