// =============================================================================
// ts_scan_ambient_tests — the header scan keeps ambient declarations apart
// from augmentations
// =============================================================================

use super::super::ts_scan::scan_ts_file_exports;

#[test]
fn an_ambient_declaration_file_exposes_its_declared_modules() {
    let exports = scan_ts_file_exports(
        "declare module 'path' { export function join(...parts: string[]): string; }\ndeclare module 'shim';\n",
        "typescript",
    );
    assert_eq!(
        exports.ambient_modules,
        vec![
            ("path".to_string(), vec!["join".to_string()]),
            ("shim".to_string(), vec![]),
        ]
    );
}

#[test]
fn a_module_file_augmenting_a_package_declares_no_module() {
    let exports = scan_ts_file_exports(
        "import type { Agent } from 'http';\nexport type Runtime = 'edge';\ndeclare module 'react' { interface ReactNode { extra?: boolean } }\n",
        "typescript",
    );
    assert!(exports.ambient_modules.is_empty());
}
