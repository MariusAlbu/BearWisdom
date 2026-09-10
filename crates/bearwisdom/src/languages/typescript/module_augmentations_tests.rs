use super::module_augmentations::{collect, interface_name, scan};

#[test]
fn scans_quoted_module_interfaces_only() {
    let source = "\
import 'vitest'\n\
declare module 'vitest' {\n\
  interface Assertion<T = any>\n\
    extends TestingLibraryMatchers<any, T> {}\n\
  interface AsymmetricMatchersContaining\n\
    extends TestingLibraryMatchers<any, any> {}\n\
}\n\
declare module Foo {\n\
  interface NotAnAugmentation {}\n\
}\n";
    let found = scan(source);
    assert!(found.contains(&("vitest".to_string(), "Assertion".to_string())));
    assert!(found.contains(&(
        "vitest".to_string(),
        "AsymmetricMatchersContaining".to_string()
    )));
    assert!(!found
        .iter()
        .any(|(_, interface)| interface == "NotAnAugmentation"));
}

#[test]
fn builds_cross_package_records_from_typescript_virtual_paths() {
    let records = collect(
        "declare module 'vitest' { interface Assertion extends Matchers {} }",
        "ext:ts:@testing-library/jest-dom/types/vitest.d.ts",
    );
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].module, "vitest");
    assert_eq!(records[0].interface, "Assertion");
    assert_eq!(
        records[0].augmenting_qname,
        "@testing-library/jest-dom.Assertion"
    );

    assert!(collect(
        "declare module '@testing-library/jest-dom' { interface Local {} }",
        "ext:ts:@testing-library/jest-dom/types/local.d.ts",
    )
    .is_empty());
}

#[test]
fn interface_names_strip_generics_and_export_keyword() {
    assert_eq!(
        interface_name("interface Assertion<T = any>"),
        Some("Assertion")
    );
    assert_eq!(interface_name("export interface Foo {"), Some("Foo"));
    assert_eq!(interface_name("not an interface line"), None);
}
