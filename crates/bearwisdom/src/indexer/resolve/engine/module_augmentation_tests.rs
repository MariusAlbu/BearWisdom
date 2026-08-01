use super::{interface_name, scan_module_augmentations};

#[test]
fn scan_module_augmentations_finds_quoted_module_interfaces() {
    // The jest-dom augmentation shape: a quoted `declare module` whose body lists
    // interfaces that extend a matcher type across multiple lines.
    let src = "\
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
    let got = scan_module_augmentations(src);
    assert!(got.contains(&("vitest".to_string(), "Assertion".to_string())));
    assert!(got.contains(&("vitest".to_string(), "AsymmetricMatchersContaining".to_string())));
    // A bare (unquoted) `declare module Foo` is a namespace, not an augmentation.
    assert!(!got.iter().any(|(_, i)| i == "NotAnAugmentation"));
}

#[test]
fn interface_name_strips_generics_and_keyword() {
    assert_eq!(interface_name("interface Assertion<T = any>"), Some("Assertion"));
    assert_eq!(interface_name("export interface Foo {"), Some("Foo"));
    assert_eq!(interface_name("  not an interface line"), None);
}
